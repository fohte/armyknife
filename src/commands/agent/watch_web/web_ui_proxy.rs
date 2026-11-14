use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::ws::{Message as ClientMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequestParts, Request, State};
use axum::http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, COOKIE, LOCATION, ORIGIN};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri};
use axum::middleware::from_fn;
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use std::future::Future;
use tokio_tungstenite::tungstenite::protocol::Message as UpstreamMessage;

use super::limits::MAX_REQUEST_BODY_BYTES;
use super::security;

const FORWARDED_REQUEST_HEADERS: [HeaderName; 8] = [
    ACCEPT,
    AUTHORIZATION,
    CONTENT_TYPE,
    COOKIE,
    HeaderName::from_static("if-modified-since"),
    HeaderName::from_static("if-none-match"),
    HeaderName::from_static("last-event-id"),
    HeaderName::from_static("range"),
];
const FORWARDED_RESPONSE_HEADERS: [HeaderName; 10] = [
    CONTENT_TYPE,
    HeaderName::from_static("cache-control"),
    HeaderName::from_static("content-encoding"),
    HeaderName::from_static("content-length"),
    HeaderName::from_static("etag"),
    HeaderName::from_static("expires"),
    HeaderName::from_static("last-modified"),
    LOCATION,
    HeaderName::from_static("retry-after"),
    HeaderName::from_static("set-cookie"),
];
const WEBSOCKET_PROTOCOL_HEADER: HeaderName = HeaderName::from_static("sec-websocket-protocol");
const FORWARDED_WEBSOCKET_HEADERS: [HeaderName; 3] =
    [AUTHORIZATION, COOKIE, WEBSOCKET_PROTOCOL_HEADER];

#[derive(Clone)]
struct WebUiProxyState<T> {
    upstream_port: u16,
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
            let headers = forwarded_response_headers(response.headers());
            Ok(WebUiResponse {
                status,
                headers,
                body: Body::from_stream(response.bytes_stream()),
            })
        }
    }
}

pub(super) fn new_client() -> anyhow::Result<Client> {
    Ok(Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}

fn forwarded_response_headers(source: &HeaderMap) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for header in FORWARDED_RESPONSE_HEADERS {
        for value in source.get_all(&header) {
            headers.append(header.clone(), value.clone());
        }
    }
    headers
}

pub(super) fn router<T: WebUiTransport>(upstream_port: u16, client: T) -> Router {
    let state = WebUiProxyState {
        upstream_port,
        client,
    };
    Router::new()
        .fallback(any(proxy_request::<T>))
        .layer(from_fn(security::validate_request))
        .with_state(state)
}

async fn proxy_request<T: WebUiTransport>(
    State(state): State<WebUiProxyState<T>>,
    request: Request,
) -> Response {
    let port = state.upstream_port;
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
    if parts.headers.contains_key(ORIGIN) {
        let Ok(origin) = HeaderValue::from_str(&format!("http://127.0.0.1:{port}")) else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        headers.insert(ORIGIN, origin);
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
    let mut headers = upstream_response.headers;
    for (name, value) in headers.iter_mut() {
        if name == LOCATION {
            rewrite_same_origin_location(value, port);
        }
    }
    let mut response = Response::new(upstream_response.body);
    *response.status_mut() = upstream_response.status;
    *response.headers_mut() = headers;
    response
}

fn rewrite_same_origin_location(location: &mut HeaderValue, port: u16) {
    let Ok(location_text) = location.to_str() else {
        return;
    };
    let Ok(location_url) = reqwest::Url::parse(location_text) else {
        return;
    };
    if location_url.scheme() != "http"
        || !matches!(location_url.host_str(), Some("127.0.0.1" | "localhost"))
        || location_url.port_or_known_default() != Some(port)
    {
        return;
    }

    let mut path_and_query = location_url.path().to_owned();
    if let Some(query) = location_url.query() {
        path_and_query.push('?');
        path_and_query.push_str(query);
    }
    if path_and_query.starts_with("//") {
        // Browsers interpret a leading double slash as a new authority.
        path_and_query.insert_str(1, "./");
    }
    if let Some(fragment) = location_url.fragment() {
        path_and_query.push('#');
        path_and_query.push_str(fragment);
    }
    if let Ok(value) = HeaderValue::from_str(&path_and_query) {
        *location = value;
    }
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
    request.headers_mut().insert(
        ORIGIN,
        HeaderValue::from_str(&format!("http://127.0.0.1:{port}"))?,
    );
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

    use axum::Router;
    use axum::body::{Body, Bytes, to_bytes};
    use axum::extract::State;
    use axum::extract::ws::{Message as ServerMessage, WebSocketUpgrade};
    use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, HOST, LOCATION, ORIGIN};
    use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode};
    use axum::response::Response;
    use axum::routing::{any, get};
    use futures_util::stream;
    use futures_util::{SinkExt, StreamExt};
    use indoc::indoc;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tower::ServiceExt;

    use super::{WebUiRequest, WebUiResponse, WebUiTransport, new_client, router};

    #[derive(Clone)]
    struct FakeTransport {
        request: Arc<Mutex<Option<WebUiRequest>>>,
        response_headers: HeaderMap,
    }

    impl WebUiTransport for FakeTransport {
        fn send(
            &self,
            request: WebUiRequest,
        ) -> impl Future<Output = anyhow::Result<WebUiResponse>> + Send {
            let mut headers = HeaderMap::new();
            headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
            headers.extend(self.response_headers.clone());
            *self
                .request
                .lock()
                .expect("observed request lock should succeed") = Some(request);
            std::future::ready(Ok(WebUiResponse {
                status: StatusCode::OK,
                headers,
                body: Body::from_stream(stream::iter([
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
                ])),
            }))
        }
    }

    #[tokio::test]
    async fn forwards_the_request_to_the_relay_specific_upstream_and_streams_sse()
    -> anyhow::Result<()> {
        let observed_request = Arc::new(Mutex::new(None));
        let app = router(
            49123,
            FakeTransport {
                request: observed_request.clone(),
                response_headers: HeaderMap::new(),
            },
        );
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/events?cursor=fixture")
                    .header(HOST, "localhost:4318")
                    .body(Body::empty())?,
            )
            .await?;
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await?;
        let request = observed_request
            .lock()
            .expect("observed request lock should succeed")
            .take()
            .expect("upstream request should be captured");

        assert_eq!(
            (
                status,
                request.method,
                request.url,
                String::from_utf8(body.to_vec())?,
            ),
            (
                StatusCode::OK,
                Method::GET,
                "http://127.0.0.1:49123/events?cursor=fixture".to_owned(),
                indoc! {"
                    event: update
                    data: ready

                "}
                .to_owned(),
            ),
        );
        Ok(())
    }

    #[tokio::test]
    async fn forwards_browser_writes_to_the_fixed_upstream() -> anyhow::Result<()> {
        let observed_request = Arc::new(Mutex::new(None));
        let app = router(
            49123,
            FakeTransport {
                request: observed_request.clone(),
                response_headers: HeaderMap::new(),
            },
        );
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/approve?item=fixture")
                    .header(HOST, "localhost:4318")
                    .header(ORIGIN, "http://localhost:4318")
                    .header(CONTENT_TYPE, "application/json")
                    .header(COOKIE, "session=fixture")
                    .header(AUTHORIZATION, "Bearer fixture-token")
                    .body(Body::from(r#"{"action":"approve"}"#))?,
            )
            .await?;
        let request = observed_request
            .lock()
            .expect("observed request lock should succeed")
            .take()
            .expect("upstream request should be captured");

        assert_eq!(
            (
                response.status(),
                request.method,
                request.url,
                request
                    .headers
                    .get(CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok()),
                request
                    .headers
                    .get(COOKIE)
                    .and_then(|value| value.to_str().ok()),
                request
                    .headers
                    .get(AUTHORIZATION)
                    .and_then(|value| value.to_str().ok()),
                request
                    .headers
                    .get(ORIGIN)
                    .and_then(|value| value.to_str().ok()),
                String::from_utf8(request.body.to_vec())?,
            ),
            (
                StatusCode::OK,
                Method::POST,
                "http://127.0.0.1:49123/api/approve?item=fixture".to_owned(),
                Some("application/json"),
                Some("session=fixture"),
                Some("Bearer fixture-token"),
                Some("http://127.0.0.1:49123"),
                r#"{"action":"approve"}"#.to_owned(),
            ),
        );
        Ok(())
    }

    #[tokio::test]
    async fn rewrites_redirects_to_stay_on_the_relay() -> anyhow::Result<()> {
        let app = router(
            49123,
            FakeTransport {
                request: Arc::new(Mutex::new(None)),
                response_headers: HeaderMap::from_iter([
                    (
                        LOCATION,
                        HeaderValue::from_static("http://127.0.0.1:49123/review?item=fixture"),
                    ),
                    (
                        HeaderName::from_static("set-cookie"),
                        HeaderValue::from_static("session=fixture; Path=/; HttpOnly"),
                    ),
                ]),
            },
        );
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/redirect")
                    .header(HOST, "localhost:4318")
                    .body(Body::empty())?,
            )
            .await?;

        assert_eq!(
            (
                response.status(),
                response
                    .headers()
                    .get(LOCATION)
                    .and_then(|value| value.to_str().ok()),
                response
                    .headers()
                    .get("set-cookie")
                    .and_then(|value| value.to_str().ok()),
            ),
            (
                StatusCode::OK,
                Some("/review?item=fixture"),
                Some("session=fixture; Path=/; HttpOnly"),
            ),
        );
        Ok(())
    }

    #[tokio::test]
    async fn streams_sse_chunks_before_the_upstream_finishes() -> anyhow::Result<()> {
        #[derive(Clone)]
        struct PendingSseTransport;

        impl WebUiTransport for PendingSseTransport {
            fn send(
                &self,
                _request: WebUiRequest,
            ) -> impl Future<Output = anyhow::Result<WebUiResponse>> + Send {
                std::future::ready(Ok(WebUiResponse {
                    status: StatusCode::OK,
                    headers: HeaderMap::new(),
                    body: Body::from_stream(
                        stream::once(async {
                            Ok::<_, Infallible>(Bytes::from_static(
                                indoc! {"
                                        data: ready

                                    "}
                                .as_bytes(),
                            ))
                        })
                        .chain(stream::pending()),
                    ),
                }))
            }
        }

        let response = router(49123, PendingSseTransport)
            .oneshot(
                Request::builder()
                    .uri("/events")
                    .header(HOST, "localhost:4318")
                    .body(Body::empty())?,
            )
            .await?;
        let chunk = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            response.into_body().into_data_stream().next(),
        )
        .await?
        .ok_or_else(|| anyhow::anyhow!("SSE stream ended before its first event"))??;

        assert_eq!(
            chunk.as_ref(),
            indoc! {"
                data: ready

            "}
            .as_bytes(),
        );
        Ok(())
    }

    #[tokio::test]
    async fn client_does_not_follow_upstream_redirects() -> anyhow::Result<()> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/redirect",
                    get(|| async {
                        (
                            StatusCode::FOUND,
                            [(LOCATION, "http://127.0.0.1:1/redirect-target")],
                            "redirect",
                        )
                    }),
                ),
            )
            .await
        });
        let response = new_client()?
            .send(WebUiRequest {
                method: Method::GET,
                url: format!("http://127.0.0.1:{port}/redirect"),
                headers: HeaderMap::new(),
                body: Bytes::new(),
            })
            .await?;
        let actual = (
            response.status,
            response
                .headers
                .get(LOCATION)
                .and_then(|value| value.to_str().ok()),
        );
        server.abort();

        assert_eq!(
            actual,
            (
                StatusCode::FOUND,
                Some("http://127.0.0.1:1/redirect-target"),
            ),
        );
        Ok(())
    }

    type ObservedHeaders = Arc<Mutex<Option<(Option<String>, Option<String>, Option<String>)>>>;

    async fn websocket_echo(
        State(observed_headers): State<ObservedHeaders>,
        headers: HeaderMap,
        ws: WebSocketUpgrade,
    ) -> Response {
        *observed_headers
            .lock()
            .expect("observed headers lock should succeed") = Some((
            headers
                .get(ORIGIN)
                .and_then(|header| header.to_str().ok())
                .map(str::to_owned),
            headers
                .get(COOKIE)
                .and_then(|header| header.to_str().ok())
                .map(str::to_owned),
            headers
                .get(AUTHORIZATION)
                .and_then(|header| header.to_str().ok())
                .map(str::to_owned),
        ));
        ws.on_upgrade(|mut socket| async move {
            if let Some(Ok(ServerMessage::Text(message))) = socket.recv().await {
                let _ = socket
                    .send(ServerMessage::Text(format!("reply:{message}").into()))
                    .await;
            }
        })
    }

    #[tokio::test]
    async fn relays_websocket_messages_and_rewrites_origin_to_the_upstream() -> anyhow::Result<()> {
        let upstream_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let upstream_port = upstream_listener.local_addr()?.port();
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
        let relay_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let relay_port = relay_listener.local_addr()?.port();
        let relay_server = tokio::spawn(async move {
            axum::serve(
                relay_listener,
                router(upstream_port, reqwest::Client::new()),
            )
            .await
        });
        let url = format!("ws://127.0.0.1:{relay_port}/socket");
        let mut request = url.into_client_request()?;
        request.headers_mut().insert(
            ORIGIN,
            HeaderValue::from_str(&format!("http://127.0.0.1:{relay_port}"))?,
        );
        request
            .headers_mut()
            .insert(COOKIE, HeaderValue::from_static("session=fixture"));
        request.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer fixture-token"),
        );
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await?;
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                "hello".into(),
            ))
            .await?;
        let reply = socket
            .next()
            .await
            .transpose()?
            .expect("reply should arrive");
        socket.close(None).await?;
        relay_server.abort();
        upstream_server.abort();

        assert_eq!(
            (
                reply.into_text()?.to_string(),
                observed_headers
                    .lock()
                    .expect("observed headers lock should succeed")
                    .clone(),
            ),
            (
                "reply:hello".to_owned(),
                Some((
                    Some(format!("http://127.0.0.1:{upstream_port}")),
                    Some("session=fixture".to_owned()),
                    Some("Bearer fixture-token".to_owned()),
                )),
            ),
        );
        Ok(())
    }
}
