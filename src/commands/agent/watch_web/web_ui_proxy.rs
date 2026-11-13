use std::future::Future;
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::ws::{Message as ClientMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequestParts, Request, State};
use axum::http::header::{ACCEPT, CONTENT_TYPE, COOKIE, HOST, SET_COOKIE};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri};
use axum::middleware::from_fn;
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use tokio_tungstenite::tungstenite::protocol::Message as UpstreamMessage;

use super::limits::MAX_REQUEST_BODY_BYTES;
use super::security;
use crate::commands::agent::types::Session;

const PORT_COOKIE: &str = "armyknife_web_ui_port";
const LEGACY_PORT_COOKIE: &str = "armyknife_crit_port";
const SELECTOR_ROUTES: [&str; 2] = ["/_web-ui", "/_crit"];
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
const WEBSOCKET_PROTOCOL_HEADER: HeaderName = HeaderName::from_static("sec-websocket-protocol");
const FORWARDED_WEBSOCKET_HEADERS: [HeaderName; 2] =
    [HeaderName::from_static("origin"), WEBSOCKET_PROTOCOL_HEADER];

#[derive(Clone)]
struct WebUiProxyState<T> {
    load_recorded_urls: Arc<dyn Fn() -> anyhow::Result<Vec<String>> + Send + Sync>,
    client: T,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WebUiRequest {
    method: Method,
    url: String,
    headers: HeaderMap,
    body: Bytes,
}

pub(super) struct WebUiResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: Body,
}

pub(super) trait WebUiTransport: Clone + Send + Sync + 'static {
    fn send(
        &self,
        request: WebUiRequest,
    ) -> impl Future<Output = anyhow::Result<WebUiResponse>> + Send;
}

impl WebUiTransport for Client {
    fn send(
        &self,
        request: WebUiRequest,
    ) -> impl Future<Output = anyhow::Result<WebUiResponse>> + Send {
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
            Ok(WebUiResponse {
                status,
                headers,
                body: Body::from_stream(response.bytes_stream()),
            })
        }
    }
}

pub(super) fn router<T: WebUiTransport>(
    load_sessions: impl Fn() -> anyhow::Result<Vec<Session>> + Send + Sync + 'static,
    client: T,
) -> Router {
    router_with_url_loader(
        move || {
            let sessions = load_sessions()?;
            Ok(sessions
                .iter()
                .flat_map(|session| {
                    session
                        .crit_urls
                        .iter()
                        .chain(session.web_uis.iter().map(|web_ui| &web_ui.url))
                })
                .cloned()
                .collect())
        },
        client,
    )
}

fn router_with_url_loader<T: WebUiTransport>(
    load_recorded_urls: impl Fn() -> anyhow::Result<Vec<String>> + Send + Sync + 'static,
    client: T,
) -> Router {
    let state = WebUiProxyState {
        load_recorded_urls: Arc::new(load_recorded_urls),
        client,
    };
    Router::new()
        .route("/_web-ui/{port}", any(select_web_ui::<T>))
        .route("/_web-ui/{port}/", any(select_web_ui::<T>))
        .route("/_web-ui/{port}/{*path}", any(select_web_ui::<T>))
        .route("/_crit/{port}", any(select_web_ui::<T>))
        .route("/_crit/{port}/", any(select_web_ui::<T>))
        .route("/_crit/{port}/{*path}", any(select_web_ui::<T>))
        .fallback(any(proxy_request::<T>))
        .layer(from_fn(security::validate_request))
        .with_state(state)
}

fn recorded_web_ui_port(url: &str) -> Option<u16> {
    super::super::web_ui::local_web_ui_port(url).ok()
}

async fn recorded_ports<T: WebUiTransport>(
    state: &WebUiProxyState<T>,
) -> Result<Vec<u16>, StatusCode> {
    let load_recorded_urls = state.load_recorded_urls.clone();
    tokio::task::spawn_blocking(move || {
        load_recorded_urls().map(|urls| {
            urls.iter()
                .filter_map(|url| recorded_web_ui_port(url))
                .collect()
        })
    })
    .await
    .map_err(|error| {
        tracing::warn!(%error, "failed to join web UI port lookup");
        StatusCode::INTERNAL_SERVER_ERROR
    })?
    .map_err(|error| {
        tracing::warn!(%error, "failed to load registered web UI ports");
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

async fn select_web_ui<T: WebUiTransport>(
    State(state): State<WebUiProxyState<T>>,
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
    let path = SELECTOR_ROUTES
        .iter()
        .find_map(|route| uri.path().strip_prefix(&format!("{route}/{port}")))?;
    let path = if path.is_empty() { "/" } else { path };
    if !path.starts_with('/') || path.starts_with("//") {
        return None;
    }
    match uri.query() {
        Some(query) => Some(format!("{path}?{query}")),
        None => Some(path.to_owned()),
    }
}

async fn proxy_request<T: WebUiTransport>(
    State(state): State<WebUiProxyState<T>>,
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
    if security::is_websocket_upgrade(request.headers()) {
        let (mut parts, _) = request.into_parts();
        let websocket = match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
            Ok(websocket) => websocket,
            Err(response) => return response.into_response(),
        };
        let upstream_request = match websocket_request(port, &uri, &parts.headers) {
            Ok(request) => request,
            Err(error) => {
                tracing::warn!(%error, port, "failed to create web UI WebSocket request");
                return StatusCode::BAD_REQUEST.into_response();
            }
        };
        let (upstream, response) = match tokio_tungstenite::connect_async(upstream_request).await {
            Ok(connection) => connection,
            Err(error) => {
                tracing::warn!(%error, port, "web UI WebSocket connection failed");
                return StatusCode::BAD_GATEWAY.into_response();
            }
        };
        let websocket = if let Some(protocol) = response
            .headers()
            .get("sec-websocket-protocol")
            .and_then(|protocol| protocol.to_str().ok())
        {
            websocket.protocols([protocol.to_owned()])
        } else {
            websocket
        };
        return websocket.on_upgrade(move |client_socket| async move {
            relay_websocket(port, client_socket, upstream).await;
        });
    }

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
        .send(WebUiRequest {
            method: parts.method,
            url: upstream_url,
            headers,
            body,
        })
        .await
    {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(%error, port, "web UI proxy request failed");
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
        (name == PORT_COOKIE || name == LEGACY_PORT_COOKIE)
            .then(|| value.parse::<u16>().ok())
            .flatten()
    })
}

fn websocket_request(
    port: u16,
    uri: &Uri,
    headers: &HeaderMap,
) -> anyhow::Result<tokio_tungstenite::tungstenite::http::Request<()>> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    let url = format!(
        "ws://127.0.0.1:{port}{}",
        uri.path_and_query().map_or("/", |value| value.as_str())
    );
    let mut request = url.into_client_request()?;
    for header in FORWARDED_WEBSOCKET_HEADERS {
        for value in headers.get_all(&header) {
            request.headers_mut().append(header.clone(), value.clone());
        }
    }
    Ok(request)
}

async fn relay_websocket(
    port: u16,
    mut client: WebSocket,
    mut upstream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) {
    loop {
        tokio::select! {
            message = client.recv() => {
                match message {
                    Some(Ok(message)) => {
                        let close = matches!(message, ClientMessage::Close(_));
                        if upstream.send(to_upstream_message(message)).await.is_err() || close {
                            return;
                        }
                    }
                    Some(Err(error)) => {
                        tracing::warn!(%error, port, "web UI client WebSocket closed with an error");
                        return;
                    }
                    None => return,
                }
            }
            message = upstream.next() => {
                match message {
                    Some(Ok(message)) => {
                        let close = matches!(message, UpstreamMessage::Close(_));
                        if let Some(message) = to_client_message(message)
                            && client.send(message).await.is_err()
                        {
                            return;
                        }
                        if close {
                            return;
                        }
                    }
                    Some(Err(error)) => {
                        tracing::warn!(%error, port, "web UI upstream WebSocket closed with an error");
                        return;
                    }
                    None => return,
                }
            }
        }
    }
}

fn to_upstream_message(message: ClientMessage) -> UpstreamMessage {
    match message {
        ClientMessage::Text(text) => UpstreamMessage::Text(text.to_string().into()),
        ClientMessage::Binary(bytes) => UpstreamMessage::Binary(bytes),
        ClientMessage::Ping(bytes) => UpstreamMessage::Ping(bytes),
        ClientMessage::Pong(bytes) => UpstreamMessage::Pong(bytes),
        ClientMessage::Close(frame) => UpstreamMessage::Close(frame.map(|frame| {
            tokio_tungstenite::tungstenite::protocol::CloseFrame {
                code: frame.code.into(),
                reason: frame.reason.to_string().into(),
            }
        })),
    }
}

fn to_client_message(message: UpstreamMessage) -> Option<ClientMessage> {
    match message {
        UpstreamMessage::Text(text) => Some(ClientMessage::Text(text.as_str().to_owned().into())),
        UpstreamMessage::Binary(bytes) => Some(ClientMessage::Binary(bytes)),
        UpstreamMessage::Ping(bytes) => Some(ClientMessage::Ping(bytes)),
        UpstreamMessage::Pong(bytes) => Some(ClientMessage::Pong(bytes)),
        UpstreamMessage::Close(frame) => Some(ClientMessage::Close(frame.map(|frame| {
            axum::extract::ws::CloseFrame {
                code: frame.code.into(),
                reason: frame.reason.to_string().into(),
            }
        }))),
        UpstreamMessage::Frame(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use axum::Router;
    use axum::body::{Body, Bytes, to_bytes};
    use axum::extract::State;
    use axum::extract::ws::{Message as ServerMessage, WebSocketUpgrade};
    use axum::http::header::{
        ACCEPT, CACHE_CONTROL, CONTENT_TYPE, COOKIE, HOST, LOCATION, ORIGIN, SET_COOKIE,
    };
    use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode};
    use axum::response::Response;
    use axum::routing::any;
    use futures_util::stream;
    use futures_util::{SinkExt, StreamExt};
    use indoc::indoc;
    use rstest::rstest;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tower::ServiceExt;

    use super::{
        WEBSOCKET_PROTOCOL_HEADER, WebUiRequest, WebUiResponse, WebUiTransport,
        router_with_url_loader,
    };

    #[derive(Clone)]
    struct FakeTransport {
        sent_request: Arc<Mutex<Option<WebUiRequest>>>,
        response_body: Arc<Mutex<Option<Body>>>,
    }

    impl WebUiTransport for FakeTransport {
        fn send(
            &self,
            request: WebUiRequest,
        ) -> impl std::future::Future<Output = anyhow::Result<WebUiResponse>> + Send {
            let sent_request = self.sent_request.clone();
            let response_body = self.response_body.clone();
            async move {
                *sent_request.lock().unwrap() = Some(request);
                let body = response_body.lock().unwrap().take().unwrap();
                let mut headers = axum::http::HeaderMap::new();
                headers.insert(CONTENT_TYPE, "text/event-stream".parse().unwrap());
                headers.insert(CACHE_CONTROL, "no-cache".parse().unwrap());
                Ok(WebUiResponse {
                    status: StatusCode::OK,
                    headers,
                    body,
                })
            }
        }
    }

    fn fake_transport() -> (FakeTransport, Arc<Mutex<Option<WebUiRequest>>>) {
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

    type ObservedWebSocketHeaders = Arc<Mutex<Option<(Option<String>, Option<String>)>>>;

    async fn websocket_echo(
        State(observed_headers): State<ObservedWebSocketHeaders>,
        headers: HeaderMap,
        ws: WebSocketUpgrade,
    ) -> Response {
        *observed_headers.lock().unwrap() = Some((
            headers
                .get(ORIGIN)
                .and_then(|header| header.to_str().ok())
                .map(str::to_owned),
            headers
                .get("sec-websocket-protocol")
                .and_then(|header| header.to_str().ok())
                .map(str::to_owned),
        ));
        ws.protocols(["chat"]).on_upgrade(|mut socket| async move {
            if let Some(Ok(ServerMessage::Text(message))) = socket.recv().await {
                let _ = socket
                    .send(ServerMessage::Text(format!("reply:{message}").into()))
                    .await;
            }
        })
    }

    #[tokio::test]
    async fn web_ui_websocket_relays_messages_in_both_directions() {
        let upstream_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let upstream_port = upstream_listener.local_addr().unwrap().port();
        let observed_headers = Arc::new(Mutex::new(None));
        let upstream_server = tokio::spawn({
            let observed_headers = observed_headers.clone();
            async move {
                axum::serve(
                    upstream_listener,
                    Router::new()
                        .route("/socket", any(websocket_echo))
                        .with_state(observed_headers),
                )
                .await
            }
        });
        let relay_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let relay_port = relay_listener.local_addr().unwrap().port();
        let recorded_url = format!("http://127.0.0.1:{upstream_port}/design/sample/");
        let relay_server = tokio::spawn(async move {
            axum::serve(
                relay_listener,
                router_with_url_loader(move || Ok(vec![recorded_url.clone()]), fake_transport().0),
            )
            .await
        });
        let mut request = format!("ws://127.0.0.1:{relay_port}/socket")
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            COOKIE,
            format!("armyknife_web_ui_port={upstream_port}")
                .parse()
                .unwrap(),
        );
        let relay_origin = format!("http://127.0.0.1:{relay_port}");
        request
            .headers_mut()
            .insert(ORIGIN, relay_origin.parse().unwrap());
        request.headers_mut().insert(
            WEBSOCKET_PROTOCOL_HEADER.clone(),
            "chat, superchat".parse().unwrap(),
        );
        let actual = tokio::time::timeout(Duration::from_secs(3), async {
            let (mut socket, handshake) = tokio_tungstenite::connect_async(request).await.unwrap();
            socket
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    "from-client".into(),
                ))
                .await
                .unwrap();
            let response = socket.next().await.unwrap().unwrap();
            (
                handshake.headers().get(&WEBSOCKET_PROTOCOL_HEADER).cloned(),
                response,
                observed_headers.lock().unwrap().clone(),
            )
        })
        .await
        .ok();

        assert_eq!(
            actual,
            Some((
                Some(HeaderValue::from_static("chat")),
                tokio_tungstenite::tungstenite::Message::Text("reply:from-client".into()),
                Some((Some(relay_origin), Some("chat, superchat".to_owned()),)),
            )),
        );
        relay_server.abort();
        upstream_server.abort();
    }

    #[rstest]
    #[case::missing_origin(None)]
    #[case::different_localhost_port(Some("http://localhost:4319"))]
    #[tokio::test]
    async fn websocket_upgrades_reject_origins_that_do_not_match_host(
        #[case] origin: Option<&str>,
    ) {
        let app = router_with_url_loader(
            || Ok(vec!["http://127.0.0.1:48761/design/sample/".to_owned()]),
            fake_transport().0,
        );
        let mut request = Request::builder()
            .uri("/socket")
            .header(HOST, "localhost:4318")
            .header(COOKIE, "armyknife_web_ui_port=48761")
            .header("upgrade", "websocket");
        if let Some(origin) = origin {
            request = request.header(ORIGIN, origin);
        }
        let response = app
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn websocket_upstream_handshake_failures_return_gateway_error() {
        let upstream_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let upstream_port = upstream_listener.local_addr().unwrap().port();
        drop(upstream_listener);
        let relay_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let relay_port = relay_listener.local_addr().unwrap().port();
        let relay_server = tokio::spawn(async move {
            axum::serve(
                relay_listener,
                router_with_url_loader(
                    move || {
                        Ok(vec![format!(
                            "http://127.0.0.1:{upstream_port}/design/sample/"
                        )])
                    },
                    fake_transport().0,
                ),
            )
            .await
        });
        let mut request = format!("ws://127.0.0.1:{relay_port}/socket")
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            ORIGIN,
            format!("http://127.0.0.1:{relay_port}").parse().unwrap(),
        );
        request.headers_mut().insert(
            COOKIE,
            format!("armyknife_web_ui_port={upstream_port}")
                .parse()
                .unwrap(),
        );
        let actual_status = tokio::time::timeout(
            Duration::from_secs(3),
            tokio_tungstenite::connect_async(request),
        )
        .await
        .ok()
        .and_then(Result::err)
        .and_then(|error| match error {
            tokio_tungstenite::tungstenite::Error::Http(response) => Some(response.status()),
            _ => None,
        });

        assert_eq!(actual_status, Some(StatusCode::BAD_GATEWAY));
        relay_server.abort();
    }

    #[rstest]
    #[case::loopback_ipv4(
        "http://127.0.0.1:48761/review/sample",
        StatusCode::SEE_OTHER,
        Some("/review/sample?mode=compact"),
        Some("armyknife_web_ui_port=48761; Path=/; HttpOnly; SameSite=Lax")
    )]
    #[case::localhost(
        "http://localhost:48761/review/sample",
        StatusCode::SEE_OTHER,
        Some("/review/sample?mode=compact"),
        Some("armyknife_web_ui_port=48761; Path=/; HttpOnly; SameSite=Lax")
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
    async fn trailing_slash_selection_redirects_to_root_with_port_cookie() {
        let app = router_with_url_loader(
            || Ok(vec!["http://localhost:48761".to_owned()]),
            fake_transport().0,
        );
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/_crit/48761/")
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
                Some("/".parse().unwrap()),
                Some(
                    "armyknife_web_ui_port=48761; Path=/; HttpOnly; SameSite=Lax"
                        .parse()
                        .unwrap()
                ),
            ),
        );
    }

    #[tokio::test]
    async fn web_ui_selection_preserves_path_and_sets_the_shared_cookie() {
        let app = router_with_url_loader(
            || Ok(vec!["http://127.0.0.1:48761/design/sample/".to_owned()]),
            fake_transport().0,
        );
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/_web-ui/48761/design/sample/?mode=edit")
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
                Some("/design/sample/?mode=edit".parse().unwrap()),
                Some(
                    "armyknife_web_ui_port=48761; Path=/; HttpOnly; SameSite=Lax"
                        .parse()
                        .unwrap()
                ),
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
        let websocket = router_with_url_loader(|| Ok(Vec::new()), fake_transport().0)
            .oneshot(
                Request::builder()
                    .uri("/socket")
                    .header(HOST, "localhost:4318")
                    .header(COOKIE, "armyknife_web_ui_port=48761")
                    .header("upgrade", "websocket")
                    .header("connection", "Upgrade")
                    .header("sec-websocket-key", "fixture-websocket-key")
                    .header("sec-websocket-version", "13")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            (selected.status(), proxied.status(), websocket.status()),
            (
                StatusCode::NOT_FOUND,
                StatusCode::NOT_FOUND,
                StatusCode::NOT_FOUND
            )
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
                WebUiRequest {
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
                Some(WebUiRequest {
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
