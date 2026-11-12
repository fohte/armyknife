use std::future::Future;
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{Request, State};
use axum::http::header::{ACCEPT, CONTENT_TYPE, COOKIE, HOST, SET_COOKIE};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri};
use axum::middleware::from_fn;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use reqwest::Client;

use super::limits::MAX_REQUEST_BODY_BYTES;
use super::security;
use crate::commands::agent::types::Session;

const PORT_COOKIE: &str = "armyknife_crit_port";
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
struct CritProxyState<T> {
    load_recorded_urls: Arc<dyn Fn() -> anyhow::Result<Vec<String>> + Send + Sync>,
    client: T,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct UpstreamRequest {
    method: Method,
    url: String,
    headers: HeaderMap,
    body: Bytes,
}

pub(super) struct UpstreamResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: Body,
}

pub(super) trait CritTransport: Clone + Send + Sync + 'static {
    fn send(
        &self,
        request: UpstreamRequest,
    ) -> impl Future<Output = anyhow::Result<UpstreamResponse>> + Send;
}

impl CritTransport for Client {
    fn send(
        &self,
        request: UpstreamRequest,
    ) -> impl Future<Output = anyhow::Result<UpstreamResponse>> + Send {
        let client = self.clone();
        async move {
            let mut upstream_request = client
                .request(request.method, request.url)
                .body(request.body);
            for (name, value) in &request.headers {
                upstream_request = upstream_request.header(name, value);
            }
            let response = upstream_request.send().await?;
            let status =
                StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let mut headers = HeaderMap::new();
            for header in FORWARDED_RESPONSE_HEADERS {
                for value in response.headers().get_all(&header) {
                    headers.append(header.clone(), value.clone());
                }
            }
            Ok(UpstreamResponse {
                status,
                headers,
                body: Body::from_stream(response.bytes_stream()),
            })
        }
    }
}

pub(super) fn router<T: CritTransport>(
    load_sessions: impl Fn() -> anyhow::Result<Vec<Session>> + Send + Sync + 'static,
    client: T,
) -> Router {
    router_with_url_loader(
        move || {
            let sessions = load_sessions()?;
            Ok(sessions
                .iter()
                .flat_map(|session| session.crit_urls.iter())
                .cloned()
                .collect())
        },
        client,
    )
}

fn router_with_url_loader<T: CritTransport>(
    load_recorded_urls: impl Fn() -> anyhow::Result<Vec<String>> + Send + Sync + 'static,
    client: T,
) -> Router {
    let state = CritProxyState {
        load_recorded_urls: Arc::new(load_recorded_urls),
        client,
    };
    Router::new()
        .route("/_crit/{port}", get(select_crit::<T>))
        .route("/_crit/{port}/{*path}", get(select_crit::<T>))
        .fallback(any(proxy_request::<T>))
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

async fn recorded_ports<T: CritTransport>(
    state: &CritProxyState<T>,
) -> Result<Vec<u16>, StatusCode> {
    let load_recorded_urls = state.load_recorded_urls.clone();
    tokio::task::spawn_blocking(move || {
        load_recorded_urls().map(|urls| {
            urls.iter()
                .filter_map(|url| recorded_crit_port(url))
                .collect()
        })
    })
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

async fn select_crit<T: CritTransport>(
    State(state): State<CritProxyState<T>>,
    request: Request,
) -> Response {
    let Some(port) = request
        .uri()
        .path()
        .split('/')
        .nth(2)
        .and_then(|port| port.parse::<u16>().ok())
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match recorded_ports(&state).await {
        Ok(ports) if ports.contains(&port) => {}
        Ok(_) => return StatusCode::NOT_FOUND.into_response(),
        Err(status) => return status.into_response(),
    }

    let Some(path_and_query) = selection_target(request.uri(), port) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let mut response = StatusCode::SEE_OTHER.into_response();
    if let Ok(location) = HeaderValue::from_str(&path_and_query) {
        response
            .headers_mut()
            .insert(axum::http::header::LOCATION, location);
    }
    let secure = request
        .headers()
        .get(HOST)
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

fn selection_target(uri: &Uri, port: u16) -> Option<String> {
    let selector = format!("/_crit/{port}");
    let path = uri.path().strip_prefix(&selector)?;
    let path = if path.is_empty() { "/" } else { path };
    if !path.starts_with('/') || path.starts_with("//") {
        return None;
    }
    match uri.query() {
        Some(query) => Some(format!("{path}?{query}")),
        None => Some(path.to_owned()),
    }
}

async fn proxy_request<T: CritTransport>(
    State(state): State<CritProxyState<T>>,
    request: Request,
) -> Response {
    let Some(port) = request
        .headers()
        .get(COOKIE)
        .and_then(|cookie| cookie.to_str().ok())
        .and_then(cookie_port)
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match recorded_ports(&state).await {
        Ok(ports) if ports.contains(&port) => {}
        Ok(_) => return StatusCode::NOT_FOUND.into_response(),
        Err(status) => return status.into_response(),
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
    let mut headers = HeaderMap::new();
    for header in FORWARDED_REQUEST_HEADERS {
        for value in parts.headers.get_all(&header) {
            headers.append(header.clone(), value.clone());
        }
    }

    let upstream_response = match state
        .client
        .send(UpstreamRequest {
            method: parts.method,
            url: upstream_url,
            headers,
            body,
        })
        .await
    {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(%error, port, "crit proxy request failed");
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };
    let mut response = Response::new(upstream_response.body);
    *response.status_mut() = upstream_response.status;
    *response.headers_mut() = upstream_response.headers;
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
    use std::convert::Infallible;
    use std::sync::{Arc, Mutex};

    use axum::body::{Body, Bytes, to_bytes};
    use axum::http::header::{
        ACCEPT, CACHE_CONTROL, CONTENT_TYPE, COOKIE, HOST, LOCATION, ORIGIN, SET_COOKIE,
    };
    use axum::http::{HeaderMap, Method, Request, StatusCode};
    use futures_util::stream;
    use indoc::indoc;
    use rstest::rstest;
    use tower::ServiceExt;

    use super::{CritTransport, UpstreamRequest, UpstreamResponse, router_with_url_loader};

    #[derive(Clone)]
    struct FakeTransport {
        sent_request: Arc<Mutex<Option<UpstreamRequest>>>,
        response_body: Arc<Mutex<Option<Body>>>,
    }

    impl CritTransport for FakeTransport {
        fn send(
            &self,
            request: UpstreamRequest,
        ) -> impl std::future::Future<Output = anyhow::Result<UpstreamResponse>> + Send {
            let sent_request = self.sent_request.clone();
            let response_body = self.response_body.clone();
            async move {
                *sent_request.lock().unwrap() = Some(request);
                let body = response_body.lock().unwrap().take().unwrap();
                let mut headers = axum::http::HeaderMap::new();
                headers.insert(CONTENT_TYPE, "text/event-stream".parse().unwrap());
                headers.insert(CACHE_CONTROL, "no-cache".parse().unwrap());
                Ok(UpstreamResponse {
                    status: StatusCode::OK,
                    headers,
                    body,
                })
            }
        }
    }

    fn fake_transport() -> (FakeTransport, Arc<Mutex<Option<UpstreamRequest>>>) {
        let sent_request = Arc::new(Mutex::new(None));
        let response_body = Body::from_stream(stream::iter([
            Ok::<_, Infallible>(Bytes::from_static(
                indoc! {"
                event: update
            "}
                .as_bytes(),
            )),
            Ok::<_, Infallible>(Bytes::from_static(
                indoc! {"
                data: ready

            "}
                .as_bytes(),
            )),
        ]));
        (
            FakeTransport {
                sent_request: sent_request.clone(),
                response_body: Arc::new(Mutex::new(Some(response_body))),
            },
            sent_request,
        )
    }

    #[rstest]
    #[case::loopback_ipv4(
        "http://127.0.0.1:48761/review/sample",
        StatusCode::SEE_OTHER,
        Some("/review/sample?mode=compact"),
        Some("armyknife_crit_port=48761; Path=/; HttpOnly; SameSite=Lax")
    )]
    #[case::localhost(
        "http://localhost:48761/review/sample",
        StatusCode::SEE_OTHER,
        Some("/review/sample?mode=compact"),
        Some("armyknife_crit_port=48761; Path=/; HttpOnly; SameSite=Lax")
    )]
    #[case::remote_host(
        "http://review.example.invalid:48761/review/sample",
        StatusCode::NOT_FOUND,
        None,
        None
    )]
    #[case::secure_scheme(
        "https://localhost:48761/review/sample",
        StatusCode::NOT_FOUND,
        None,
        None
    )]
    #[case::missing_port("http://localhost/review/sample", StatusCode::NOT_FOUND, None, None)]
    #[tokio::test]
    async fn selection_only_accepts_recorded_loopback_http_urls(
        #[case] recorded_url: &str,
        #[case] expected_status: StatusCode,
        #[case] expected_location: Option<&str>,
        #[case] expected_cookie: Option<&str>,
    ) {
        let recorded_url = recorded_url.to_owned();
        let app =
            router_with_url_loader(move || Ok(vec![recorded_url.clone()]), fake_transport().0);
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
                expected_status,
                expected_location.map(|location| location.parse().unwrap()),
                expected_cookie.map(|cookie| cookie.parse().unwrap()),
            ),
        );
    }

    #[tokio::test]
    async fn unrecorded_ports_are_not_selected_or_proxied() {
        let app = router_with_url_loader(|| Ok(Vec::new()), fake_transport().0);
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
                    .header(COOKIE, "armyknife_crit_port=48761")
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
    async fn crit_requests_forward_to_a_recorded_port_and_stream_the_response() {
        let (transport, sent_request) = fake_transport();
        let app = router_with_url_loader(
            || Ok(vec!["http://localhost:48761/review/sample".to_owned()]),
            transport,
        );
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/comments?review=main")
                    .header(HOST, "localhost:4318")
                    .header(ORIGIN, "http://localhost:4318")
                    .header(COOKIE, "armyknife_crit_port=48761")
                    .header(CONTENT_TYPE, "application/json")
                    .header(ACCEPT, "text/event-stream")
                    .header("last-event-id", "fixture-event")
                    .body(Body::from("{\"message\":\"fixture comment\"}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let response_status = response.status();
        let response_headers = (
            response.headers().get(CONTENT_TYPE).cloned(),
            response.headers().get(CACHE_CONTROL).cloned(),
        );
        let response_body = to_bytes(response.into_body(), 1024).await.unwrap();
        let sent_request = sent_request.lock().unwrap().take().unwrap();
        let mut expected_request_headers = HeaderMap::new();
        expected_request_headers.insert(CONTENT_TYPE, "application/json".parse().unwrap());
        expected_request_headers.insert(ACCEPT, "text/event-stream".parse().unwrap());
        expected_request_headers.insert("last-event-id", "fixture-event".parse().unwrap());

        assert_eq!(
            (
                sent_request,
                response_status,
                response_headers,
                response_body,
            ),
            (
                UpstreamRequest {
                    method: Method::POST,
                    url: "http://127.0.0.1:48761/api/comments?review=main".into(),
                    headers: expected_request_headers,
                    body: Bytes::from_static(b"{\"message\":\"fixture comment\"}"),
                },
                StatusCode::OK,
                (
                    Some("text/event-stream".parse().unwrap()),
                    Some("no-cache".parse().unwrap()),
                ),
                Bytes::from_static(
                    indoc! {"
                        event: update
                        data: ready

                    "}
                    .as_bytes(),
                ),
            ),
        );
    }

    #[tokio::test]
    async fn attachment_requests_larger_than_two_mebibytes_are_forwarded() {
        let upload_body = Bytes::from(vec![b'x'; 3 * 1024 * 1024]);
        let expected_body = upload_body.clone();
        let (transport, sent_request) = fake_transport();
        let app = router_with_url_loader(
            || Ok(vec!["http://localhost:48761/review/sample".to_owned()]),
            transport,
        );
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/attachments")
                    .header(HOST, "localhost:4318")
                    .header(ORIGIN, "http://localhost:4318")
                    .header(COOKIE, "armyknife_crit_port=48761")
                    .header(CONTENT_TYPE, "application/octet-stream")
                    .body(Body::from(upload_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let response_status = response.status();
        let sent_request = sent_request.lock().unwrap().take();
        let mut expected_request_headers = HeaderMap::new();
        expected_request_headers.insert(CONTENT_TYPE, "application/octet-stream".parse().unwrap());

        assert_eq!(
            (response_status, sent_request),
            (
                StatusCode::OK,
                Some(UpstreamRequest {
                    method: Method::POST,
                    url: "http://127.0.0.1:48761/api/attachments".into(),
                    headers: expected_request_headers,
                    body: expected_body,
                }),
            ),
        );
    }

    #[tokio::test]
    async fn double_slash_selection_paths_are_rejected() {
        let app = router_with_url_loader(
            || Ok(vec!["http://localhost:48761/review/sample".to_owned()]),
            fake_transport().0,
        );
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/_crit/48761//review/sample")
                    .header(HOST, "localhost:4318")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn session_store_failures_are_reported_as_server_errors() {
        let app = router_with_url_loader(
            || Err(anyhow::anyhow!("fixture session store failure")),
            fake_transport().0,
        );
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
                    .header(COOKIE, "armyknife_crit_port=48761")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            (selected.status(), proxied.status()),
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                StatusCode::INTERNAL_SERVER_ERROR
            )
        );
    }

    #[tokio::test]
    async fn requests_from_unapproved_hosts_are_rejected() {
        let app = router_with_url_loader(|| Ok(Vec::new()), fake_transport().0);
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
