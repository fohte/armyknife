use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::header::{ACCEPT, CONTENT_TYPE, COOKIE, SET_COOKIE};
use axum::http::{HeaderName, HeaderValue, StatusCode, Uri};
use axum::middleware::from_fn;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use reqwest::Client;

use super::security;
use crate::commands::agent::types::Session;

const PORT_COOKIE: &str = "armyknife_crit_port";
const MAX_REQUEST_BODY_BYTES: usize = 2 * 1024 * 1024;
const FORWARDED_REQUEST_HEADERS: [HeaderName; 6] = [
    ACCEPT,
    CONTENT_TYPE,
    HeaderName::from_static("if-modified-since"),
    HeaderName::from_static("if-none-match"),
    HeaderName::from_static("last-event-id"),
    HeaderName::from_static("range"),
];
const FORWARDED_RESPONSE_HEADERS: [HeaderName; 8] = [
    CONTENT_TYPE,
    HeaderName::from_static("cache-control"),
    HeaderName::from_static("content-encoding"),
    HeaderName::from_static("content-length"),
    HeaderName::from_static("etag"),
    HeaderName::from_static("expires"),
    HeaderName::from_static("last-modified"),
    HeaderName::from_static("retry-after"),
];

#[derive(Clone)]
struct CritProxyState {
    load_recorded_ports: Arc<dyn Fn() -> anyhow::Result<Vec<u16>> + Send + Sync>,
    client: Client,
}

pub(super) fn router(
    load_sessions: impl Fn() -> anyhow::Result<Vec<Session>> + Send + Sync + 'static,
    client: Client,
) -> Router {
    router_with_port_loader(
        move || {
            let sessions = load_sessions()?;
            Ok(sessions
                .iter()
                .flat_map(|session| session.crit_urls.iter())
                .filter_map(|url| recorded_crit_port(url))
                .collect())
        },
        client,
    )
}

fn router_with_port_loader(
    load_recorded_ports: impl Fn() -> anyhow::Result<Vec<u16>> + Send + Sync + 'static,
    client: Client,
) -> Router {
    let state = CritProxyState {
        load_recorded_ports: Arc::new(load_recorded_ports),
        client,
    };
    Router::new()
        .route("/_crit/{port}", get(select_crit))
        .route("/_crit/{port}/{*path}", get(select_crit))
        .fallback(any(proxy_request))
        .layer(from_fn(security::validate_request))
        .with_state(state)
}

fn recorded_crit_port(url: &str) -> Option<u16> {
    let url = reqwest::Url::parse(url).ok()?;
    (url.scheme() == "http"
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(url.host_str(), Some("localhost" | "127.0.0.1")))
    .then_some(url.port())
    .flatten()
    .filter(|port| *port != 0)
}

async fn recorded_ports(state: &CritProxyState) -> Result<Vec<u16>, StatusCode> {
    let load_recorded_ports = state.load_recorded_ports.clone();
    tokio::task::spawn_blocking(move || load_recorded_ports())
        .await
        .map_err(|error| {
            tracing::warn!(%error, "failed to join crit proxy port lookup");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .map_err(|error| {
            tracing::warn!(%error, "failed to load crit proxy ports");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn select_crit(State(state): State<CritProxyState>, request: Request) -> Response {
    let Some(port) = request
        .uri()
        .path()
        .split('/')
        .nth(2)
        .and_then(|port| port.parse::<u16>().ok())
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !matches!(recorded_ports(&state).await, Ok(ports) if ports.contains(&port)) {
        return StatusCode::NOT_FOUND.into_response();
    }

    let path_and_query = selection_target(request.uri(), port);
    let mut response = StatusCode::SEE_OTHER.into_response();
    if let Ok(location) = HeaderValue::from_str(&path_and_query) {
        response
            .headers_mut()
            .insert(axum::http::header::LOCATION, location);
    }
    let secure = request
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|host| host.to_str().ok())
        .is_some_and(|host| {
            host.split(':')
                .next()
                .is_some_and(|host| host.ends_with(".ts.net"))
        });
    let cookie = if secure {
        format!("{PORT_COOKIE}={port}; Path=/; HttpOnly; SameSite=Lax; Secure")
    } else {
        format!("{PORT_COOKIE}={port}; Path=/; HttpOnly; SameSite=Lax")
    };
    if let Ok(cookie) = HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(SET_COOKIE, cookie);
    }
    response
}

fn selection_target(uri: &Uri, port: u16) -> String {
    let selector = format!("/_crit/{port}");
    let path = uri.path().strip_prefix(&selector).unwrap_or("/");
    let path = if path.is_empty() { "/" } else { path };
    match uri.query() {
        Some(query) => format!("{path}?{query}"),
        None => path.to_owned(),
    }
}

async fn proxy_request(State(state): State<CritProxyState>, request: Request) -> Response {
    let Some(port) = request
        .headers()
        .get(COOKIE)
        .and_then(|cookie| cookie.to_str().ok())
        .and_then(cookie_port)
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !matches!(recorded_ports(&state).await, Ok(ports) if ports.contains(&port)) {
        return StatusCode::NOT_FOUND.into_response();
    }

    let uri = request.uri().clone();
    let (parts, body) = request.into_parts();
    let body = match to_bytes(body, MAX_REQUEST_BODY_BYTES).await {
        Ok(body) => body,
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    let upstream_url = format!(
        "http://127.0.0.1:{port}{}",
        uri.path_and_query()
            .map_or("/", |path_and_query| path_and_query.as_str())
    );
    let mut upstream_request = state.client.request(parts.method, upstream_url).body(body);
    for header in FORWARDED_REQUEST_HEADERS {
        for value in parts.headers.get_all(&header) {
            upstream_request = upstream_request.header(&header, value);
        }
    }

    let upstream_response = match upstream_request.send().await {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(%error, port, "crit proxy request failed");
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };
    let status = StatusCode::from_u16(upstream_response.status().as_u16())
        .unwrap_or(StatusCode::BAD_GATEWAY);
    let headers = upstream_response.headers().clone();
    let body = Body::from_stream(upstream_response.bytes_stream());
    let mut response = Response::new(body);
    *response.status_mut() = status;
    for header in FORWARDED_RESPONSE_HEADERS {
        for value in headers.get_all(&header) {
            response.headers_mut().append(header.clone(), value.clone());
        }
    }
    response
}

fn cookie_port(cookies: &str) -> Option<u16> {
    cookies.split(';').find_map(|cookie| {
        let (name, value) = cookie.trim().split_once('=')?;
        (name == PORT_COOKIE)
            .then(|| value.parse::<u16>().ok())
            .flatten()
    })
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::header::{HOST, LOCATION, SET_COOKIE};
    use axum::http::{Request, StatusCode};
    use rstest::rstest;
    use tower::ServiceExt;

    use super::{cookie_port, recorded_crit_port, router_with_port_loader, selection_target};

    fn client() -> reqwest::Client {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
    }

    #[rstest]
    #[case::loopback_ipv4("http://127.0.0.1:48761/review/sample", Some(48761))]
    #[case::localhost("http://localhost:48762/review/sample", Some(48762))]
    #[case::remote_host("http://review.example.invalid:48761/review/sample", None)]
    #[case::secure_scheme("https://localhost:48761/review/sample", None)]
    #[case::missing_port("http://localhost/review/sample", None)]
    #[case::userinfo("http://user@localhost:48761/review/sample", None)]
    fn only_http_loopback_urls_with_explicit_ports_are_recorded(
        #[case] url: &str,
        #[case] expected: Option<u16>,
    ) {
        assert_eq!(recorded_crit_port(url), expected);
    }

    #[rstest]
    #[case::path(
        "/_crit/48761/review/sample?mode=compact",
        48761,
        "/review/sample?mode=compact"
    )]
    #[case::root("/_crit/48761", 48761, "/")]
    fn selection_preserves_the_crit_path_and_query(
        #[case] uri: &str,
        #[case] port: u16,
        #[case] expected: &str,
    ) {
        assert_eq!(selection_target(&uri.parse().unwrap(), port), expected);
    }

    #[rstest]
    #[case::matching_cookie("theme=dark; armyknife_crit_port=48761", Some(48761))]
    #[case::missing_cookie("theme=dark", None)]
    #[case::malformed_cookie("armyknife_crit_port=nope", None)]
    fn cookie_selection_requires_a_valid_port(
        #[case] cookies: &str,
        #[case] expected: Option<u16>,
    ) {
        assert_eq!(cookie_port(cookies), expected);
    }

    #[tokio::test]
    async fn recorded_ports_are_selected_with_a_same_origin_redirect() {
        let app = router_with_port_loader(|| Ok(vec![48761]), client());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/_crit/48761/review/sample?mode=compact")
                    .header(HOST, "localhost:4318")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            (
                response.status(),
                response.headers().get(LOCATION).cloned(),
                response.headers().get(SET_COOKIE).cloned(),
            ),
            (
                StatusCode::SEE_OTHER,
                Some("/review/sample?mode=compact".parse().unwrap()),
                Some(
                    "armyknife_crit_port=48761; Path=/; HttpOnly; SameSite=Lax"
                        .parse()
                        .unwrap()
                ),
            ),
        );
    }

    #[tokio::test]
    async fn unrecorded_ports_are_not_selected_or_proxied() {
        let app = router_with_port_loader(|| Ok(Vec::new()), client());
        let selected = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/_crit/48761/review/sample")
                    .header(HOST, "localhost:4318")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let proxied = app
            .oneshot(
                Request::builder()
                    .uri("/review/sample")
                    .header(HOST, "localhost:4318")
                    .header("cookie", "armyknife_crit_port=48761")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            (selected.status(), proxied.status()),
            (StatusCode::NOT_FOUND, StatusCode::NOT_FOUND)
        );
    }

    #[tokio::test]
    async fn requests_from_unapproved_hosts_are_rejected() {
        let app = router_with_port_loader(|| Ok(Vec::new()), client());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/review/sample")
                    .header(HOST, "review.example.invalid:4318")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
