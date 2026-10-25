use std::net::TcpStream;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use reqwest::header::{AUTHORIZATION, HeaderMap};
use serde_json::Value;
use tungstenite::client::IntoClientRequest;
use tungstenite::handshake::server::{
    ErrorResponse, Request as ServerRequest, Response as ServerResponse,
};
use tungstenite::protocol::frame::CloseFrame;
use tungstenite::protocol::{Message, WebSocket, WebSocketConfig};
use tungstenite::{accept_hdr_with_config, client_tls_with_config};

use super::RelayState;
use super::http::{
    rewrite_request_body, upstream_headers, validate_target, value_contains_tool_call,
};

const UPSTREAM_WS_URL: &str = "wss://chatgpt.com/backend-api/codex/responses";
const UPSTREAM_HOST: &str = "chatgpt.com";
const UPSTREAM_PORT: u16 = 443;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const SOCKET_POLL_INTERVAL: Duration = Duration::from_millis(250);
const MAX_WS_MESSAGE_BYTES: usize = 64 * 1024 * 1024;
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(25);

pub(super) fn handle(stream: TcpStream, state: &RelayState) -> Result<()> {
    stream
        .set_read_timeout(Some(CONNECT_TIMEOUT))
        .context("set Codex WebSocket handshake read timeout")?;
    stream
        .set_write_timeout(Some(CONNECT_TIMEOUT))
        .context("set Codex WebSocket handshake write timeout")?;
    let (mut child, headers) = accept_child(stream, state)?;
    child
        .get_mut()
        .set_read_timeout(Some(SOCKET_POLL_INTERVAL))
        .context("set Codex WebSocket child read timeout")?;
    child
        .get_mut()
        .set_write_timeout(Some(CONNECT_TIMEOUT))
        .context("set Codex WebSocket child write timeout")?;
    let mut upstream = connect_upstream(&headers, state)?;
    bridge(
        &mut child,
        &mut upstream,
        &state.parent_session_id,
        &state.cancelled,
    )
}

#[expect(
    clippy::result_large_err,
    reason = "tungstenite requires an HTTP response as the handshake rejection type"
)]
fn accept_child(
    stream: TcpStream,
    state: &RelayState,
) -> Result<(WebSocket<TcpStream>, HeaderMap)> {
    let request_headers = std::sync::Arc::new(Mutex::new(None));
    let callback_headers = request_headers.clone();
    let expected_path = state.path.clone();
    let expected_authority = state.authority.clone();
    let config = websocket_config();
    let child = accept_hdr_with_config(
        stream,
        move |request: &ServerRequest, response: ServerResponse| {
            let validation = validate_target(
                request
                    .uri()
                    .path_and_query()
                    .map(|path| path.as_str())
                    .unwrap_or_else(|| request.uri().path()),
                request.headers(),
                &expected_path,
                &expected_authority,
            );
            if validation.is_err() || !request.headers().contains_key(AUTHORIZATION) {
                return Err(rejection_response());
            }
            *callback_headers.lock().map_err(|_| rejection_response())? =
                Some(request.headers().clone());
            Ok(response)
        },
        Some(config),
    )
    .context("accept Codex status WebSocket")?;
    let headers = request_headers
        .lock()
        .map_err(|_| anyhow::anyhow!("Codex WebSocket request headers were poisoned"))?
        .clone()
        .context("Codex status WebSocket has no request headers")?;
    Ok((child, headers))
}

fn connect_upstream(
    child_headers: &HeaderMap,
    state: &RelayState,
) -> Result<WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>> {
    let mut request = UPSTREAM_WS_URL
        .into_client_request()
        .context("build official Codex WebSocket request")?;
    apply_upstream_headers(&mut request, child_headers, &state.parent_session_id)?;
    let stream = connect_tcp(state)?;
    stream
        .set_read_timeout(Some(CONNECT_TIMEOUT))
        .context("set Codex WebSocket handshake read timeout")?;
    stream
        .set_write_timeout(Some(CONNECT_TIMEOUT))
        .context("set Codex WebSocket handshake write timeout")?;
    let (mut upstream, _) = client_tls_with_config(request, stream, Some(websocket_config()), None)
        .map_err(|error| anyhow::anyhow!(error))
        .context("connect to official Codex WebSocket endpoint")?;
    set_upstream_read_timeout(&mut upstream, SOCKET_POLL_INTERVAL)?;
    Ok(upstream)
}

fn apply_upstream_headers(
    request: &mut tungstenite::http::Request<()>,
    child_headers: &HeaderMap,
    parent_session_id: &str,
) -> Result<()> {
    let headers = upstream_headers(child_headers, parent_session_id)?;
    for (name, value) in &headers {
        request.headers_mut().insert(name, value.clone());
    }
    Ok(())
}

fn connect_tcp(state: &RelayState) -> Result<TcpStream> {
    let cancelled = state.cancelled.clone();
    let stream = state.runtime.block_on(async move {
        tokio::select! {
            result = tokio::time::timeout(
                CONNECT_TIMEOUT,
                tokio::net::TcpStream::connect((UPSTREAM_HOST, UPSTREAM_PORT)),
            ) => result
                .context("official Codex WebSocket connection timed out")?
                .context("connect to official Codex WebSocket endpoint"),
            () = wait_until_cancelled(cancelled) => {
                bail!("Codex status WebSocket was cancelled while connecting")
            }
        }
    })?;
    let stream = stream
        .into_std()
        .context("convert official Codex WebSocket connection")?;
    stream
        .set_nonblocking(false)
        .context("set official Codex WebSocket connection blocking")?;
    stream
        .set_nodelay(true)
        .context("set official Codex WebSocket TCP no-delay")?;
    Ok(stream)
}

async fn wait_until_cancelled(cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    while !cancelled.load(Ordering::Relaxed) {
        tokio::time::sleep(CANCEL_POLL_INTERVAL).await;
    }
}

trait RelaySocket {
    fn read_message(&mut self) -> tungstenite::Result<Message>;
    fn send_message(&mut self, message: Message) -> tungstenite::Result<()>;
    fn close_with(&mut self, frame: Option<CloseFrame>) -> tungstenite::Result<()>;
}

impl<Stream> RelaySocket for WebSocket<Stream>
where
    Stream: std::io::Read + std::io::Write,
{
    fn read_message(&mut self) -> tungstenite::Result<Message> {
        self.read()
    }

    fn send_message(&mut self, message: Message) -> tungstenite::Result<()> {
        self.send(message)
    }

    fn close_with(&mut self, frame: Option<CloseFrame>) -> tungstenite::Result<()> {
        self.close(frame)
    }
}

fn bridge<Child, Upstream>(
    child: &mut Child,
    upstream: &mut Upstream,
    parent_session_id: &str,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<()>
where
    Child: RelaySocket,
    Upstream: RelaySocket,
{
    let mut awaiting_response = false;
    while !cancelled.load(Ordering::Relaxed) {
        if awaiting_response {
            match read_message(upstream, cancelled)? {
                Message::Text(text) => {
                    let event: Value = serde_json::from_str(text.as_str())
                        .context("parse official Codex WebSocket event before forwarding")?;
                    ensure!(
                        !value_contains_tool_call(&event),
                        "official Codex WebSocket response requested a tool call"
                    );
                    let event_type = event
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    child
                        .send_message(Message::Text(text))
                        .context("forward official Codex WebSocket event")?;
                    if is_response_terminal_event(event_type) {
                        awaiting_response = false;
                    }
                }
                Message::Ping(payload) => upstream
                    .send_message(Message::Pong(payload))
                    .context("answer official Codex WebSocket ping")?,
                Message::Pong(_) => {}
                Message::Close(frame) => {
                    let _ = child.close_with(frame);
                    return Ok(());
                }
                Message::Binary(_) | Message::Frame(_) => {
                    bail!("official Codex WebSocket returned a non-text event")
                }
            }
        } else {
            match read_message(child, cancelled)? {
                Message::Text(text) => {
                    let body = rewrite_request_body(text.as_bytes(), parent_session_id)?;
                    let value: Value = serde_json::from_slice(&body)
                        .context("parse Codex WebSocket request after rewrite")?;
                    ensure!(
                        value.get("type").and_then(Value::as_str) == Some("response.create"),
                        "Codex status WebSocket sent an unsupported request event"
                    );
                    upstream
                        .send_message(Message::Text(
                            String::from_utf8(body)
                                .context("Codex WebSocket request is not UTF-8")?
                                .into(),
                        ))
                        .context("forward Codex status WebSocket request")?;
                    awaiting_response = true;
                }
                Message::Ping(payload) => child
                    .send_message(Message::Pong(payload))
                    .context("answer Codex status WebSocket ping")?,
                Message::Pong(_) => {}
                Message::Close(frame) => {
                    let _ = upstream.close_with(frame);
                    return Ok(());
                }
                Message::Binary(_) | Message::Frame(_) => {
                    bail!("Codex status WebSocket sent a non-text request")
                }
            }
        }
    }
    let _ = child.close_with(None);
    let _ = upstream.close_with(None);
    Ok(())
}

fn read_message<Socket>(
    socket: &mut Socket,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<Message>
where
    Socket: RelaySocket,
{
    loop {
        if cancelled.load(Ordering::Relaxed) {
            bail!("Codex status WebSocket was cancelled");
        }
        match socket.read_message() {
            Ok(message) => {
                ensure!(
                    message.len() <= MAX_WS_MESSAGE_BYTES,
                    "Codex status WebSocket message exceeds the size limit"
                );
                return Ok(message);
            }
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                thread::sleep(SOCKET_POLL_INTERVAL);
            }
            Err(error) => return Err(error).context("read Codex status WebSocket message"),
        }
    }
}

fn is_response_terminal_event(event_type: &str) -> bool {
    matches!(
        event_type,
        "response.completed" | "response.failed" | "response.incomplete" | "error"
    )
}

fn websocket_config() -> WebSocketConfig {
    let mut config = WebSocketConfig::default();
    config.max_message_size = Some(MAX_WS_MESSAGE_BYTES);
    config.max_frame_size = Some(MAX_WS_MESSAGE_BYTES);
    config.max_write_buffer_size = MAX_WS_MESSAGE_BYTES;
    config
}

fn set_upstream_read_timeout(
    socket: &mut WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>,
    timeout: Duration,
) -> Result<()> {
    match socket.get_mut() {
        tungstenite::stream::MaybeTlsStream::Rustls(stream) => stream
            .sock
            .set_read_timeout(Some(timeout))
            .context("set Codex WebSocket read timeout"),
        tungstenite::stream::MaybeTlsStream::Plain(stream) => stream
            .set_read_timeout(Some(timeout))
            .context("set Codex WebSocket read timeout"),
        _ => bail!("unsupported Codex WebSocket TLS transport"),
    }
}

fn rejection_response() -> ErrorResponse {
    let mut response = ErrorResponse::new(Some("rejected".to_string()));
    *response.status_mut() = tungstenite::http::StatusCode::BAD_REQUEST;
    response
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
    use rstest::rstest;
    use serde_json::json;
    use tungstenite::client::IntoClientRequest;

    use super::{
        RelaySocket, UPSTREAM_WS_URL, apply_upstream_headers, bridge, is_response_terminal_event,
    };

    struct FakeSocket {
        incoming: VecDeque<tungstenite::Result<tungstenite::protocol::Message>>,
        sent: Vec<tungstenite::protocol::Message>,
    }

    impl FakeSocket {
        fn new(incoming: impl IntoIterator<Item = tungstenite::protocol::Message>) -> Self {
            Self {
                incoming: incoming.into_iter().map(Ok).collect(),
                sent: Vec::new(),
            }
        }
    }

    impl RelaySocket for FakeSocket {
        fn read_message(&mut self) -> tungstenite::Result<tungstenite::protocol::Message> {
            self.incoming
                .pop_front()
                .unwrap_or(Err(tungstenite::Error::ConnectionClosed))
        }

        fn send_message(
            &mut self,
            message: tungstenite::protocol::Message,
        ) -> tungstenite::Result<()> {
            self.sent.push(message);
            Ok(())
        }

        fn close_with(
            &mut self,
            _frame: Option<tungstenite::protocol::frame::CloseFrame>,
        ) -> tungstenite::Result<()> {
            Ok(())
        }
    }

    #[rstest]
    #[case::completed("response.completed", true)]
    #[case::failed("response.failed", true)]
    #[case::incomplete("response.incomplete", true)]
    #[case::progress("response.output_text.delta", false)]
    fn response_events_end_only_at_terminal_status(
        #[case] event_type: &str,
        #[case] expected: bool,
    ) {
        let event = json!({ "type": event_type });
        assert_eq!(
            is_response_terminal_event(event["type"].as_str().unwrap_or_default()),
            expected,
        );
    }

    #[test]
    fn upstream_websocket_headers_keep_generated_handshake_fields() {
        let child_headers = HeaderMap::from_iter([
            (
                AUTHORIZATION,
                HeaderValue::from_static("Bearer fixture-token"),
            ),
            (
                HeaderName::from_static("session-id"),
                HeaderValue::from_static("fixture-child-session"),
            ),
            (
                HeaderName::from_static("thread-id"),
                HeaderValue::from_static("fixture-child-thread"),
            ),
        ]);
        let mut request = UPSTREAM_WS_URL
            .into_client_request()
            .expect("fixture URL should create a WebSocket request");
        let mut expected = request.headers().clone();
        expected.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer fixture-token"),
        );
        expected.insert(
            HeaderName::from_static("session-id"),
            HeaderValue::from_static("fixture-parent-session"),
        );
        expected.insert(
            HeaderName::from_static("thread-id"),
            HeaderValue::from_static("fixture-child-thread"),
        );

        apply_upstream_headers(&mut request, &child_headers, "fixture-parent-session")
            .expect("fixture headers should be valid");

        assert_eq!(request.headers(), &expected);
    }

    #[test]
    fn websocket_bridge_rewrites_requests_and_rejects_tool_call_responses() {
        let request = serde_json::json!({
            "type": "response.create",
            "tool_choice": "auto",
            "prompt_cache_key": "fixture-child-session",
            "input": [{ "role": "user", "content": "fixture input" }],
        });
        let response = serde_json::json!({
            "type": "response.output_item.added",
            "item": { "type": "function_call" },
        });
        let mut child = FakeSocket::new([tungstenite::protocol::Message::Text(
            serde_json::to_string(&request)
                .expect("fixture request should serialize")
                .into(),
        )]);
        let mut upstream = FakeSocket::new([tungstenite::protocol::Message::Text(
            serde_json::to_string(&response)
                .expect("fixture response should serialize")
                .into(),
        )]);
        let cancelled = std::sync::atomic::AtomicBool::new(false);
        let result = bridge(
            &mut child,
            &mut upstream,
            "fixture-parent-session",
            &cancelled,
        );
        let forwarded = |messages: &[tungstenite::protocol::Message]| {
            messages
                .iter()
                .filter_map(|message| match message {
                    tungstenite::protocol::Message::Text(text) => {
                        serde_json::from_str::<serde_json::Value>(text.as_str()).ok()
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };

        assert_eq!(
            (
                result.is_err(),
                forwarded(&upstream.sent),
                forwarded(&child.sent),
            ),
            (
                true,
                vec![serde_json::json!({
                    "type": "response.create",
                    "tool_choice": "none",
                    "prompt_cache_key": "fixture-parent-session",
                    "input": [{ "role": "user", "content": "fixture input" }],
                })],
                Vec::new(),
            ),
        );
    }
}
