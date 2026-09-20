//! Delivers a message to a Codex thread through the persistent app-server.
//!
//! The control socket speaks WebSocket rather than newline-delimited JSON.
//! `turn/start` uses Codex's `StartOrSteer` input mode, so the same request
//! starts a turn for an idle thread or injects input into its active turn.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, anyhow};
use serde_json::{Value, json};
use thiserror::Error;
use tungstenite::{Message, WebSocket, client};

const INITIALIZE_REQUEST_ID: u64 = 1;
const TURN_START_REQUEST_ID: u64 = 2;
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);

pub type Result<T> = std::result::Result<T, DeliveryError>;

#[derive(Debug, Error)]
pub enum DeliveryError {
    /// The app-server did not accept `turn/start`, so queue fallback is safe.
    #[error("{0:#}")]
    NotDelivered(anyhow::Error),
    /// `turn/start` was written but no authoritative response arrived.
    #[error("Codex app-server delivery is unconfirmed; not queueing to avoid a duplicate: {0:#}")]
    Unconfirmed(anyhow::Error),
}

#[derive(Debug, Error)]
enum RequestError {
    #[error("{0:#}")]
    Rejected(anyhow::Error),
    #[error("{0:#}")]
    Transport(anyhow::Error),
}

struct RpcRequest {
    id: u64,
    method: &'static str,
    payload: Value,
}

/// Sends `content` to the Codex thread identified by `thread_id`.
pub fn send_message(thread_id: &str, content: &str) -> Result<()> {
    let socket_path = control_socket_path().map_err(DeliveryError::NotDelivered)?;
    send_message_to(&socket_path, thread_id, content)
}

fn control_socket_path() -> anyhow::Result<PathBuf> {
    let codex_home = std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::shared::dirs::home_dir().map(|home| home.join(".codex")))
        .context("could not determine CODEX_HOME for the app-server control socket")?;
    Ok(codex_home
        .join("app-server-control")
        .join("app-server-control.sock"))
}

fn send_message_to(socket_path: &Path, thread_id: &str, content: &str) -> Result<()> {
    let stream = UnixStream::connect(socket_path)
        .with_context(|| {
            format!(
                "failed to connect to Codex app-server control socket {}",
                socket_path.display()
            )
        })
        .map_err(DeliveryError::NotDelivered)?;
    stream
        .set_read_timeout(Some(RESPONSE_TIMEOUT))
        .context("failed to set Codex app-server read timeout")
        .map_err(DeliveryError::NotDelivered)?;
    stream
        .set_write_timeout(Some(RESPONSE_TIMEOUT))
        .context("failed to set Codex app-server write timeout")
        .map_err(DeliveryError::NotDelivered)?;

    let (mut socket, _) = client("ws://localhost/rpc", stream)
        .map_err(|error| anyhow!(error))
        .context("Codex app-server WebSocket handshake failed")
        .map_err(DeliveryError::NotDelivered)?;

    send_request(&mut socket, &initialize_request()).map_err(|error| match error {
        RequestError::Rejected(error) | RequestError::Transport(error) => {
            DeliveryError::NotDelivered(error)
        }
    })?;
    send_request(&mut socket, &turn_start_request(thread_id, content)).map_err(
        |error| match error {
            RequestError::Rejected(error) => DeliveryError::NotDelivered(error),
            RequestError::Transport(error) => DeliveryError::Unconfirmed(error),
        },
    )
}

fn initialize_request() -> RpcRequest {
    RpcRequest {
        id: INITIALIZE_REQUEST_ID,
        method: "initialize",
        payload: json!({
            "id": INITIALIZE_REQUEST_ID,
            "method": "initialize",
            "params": {
                "clientInfo": {
                    "name": "armyknife",
                    "title": "armyknife",
                    "version": env!("CARGO_PKG_VERSION"),
                },
            },
        }),
    }
}

fn turn_start_request(thread_id: &str, content: &str) -> RpcRequest {
    RpcRequest {
        id: TURN_START_REQUEST_ID,
        method: "turn/start",
        payload: json!({
            "id": TURN_START_REQUEST_ID,
            "method": "turn/start",
            "params": {
                "threadId": thread_id,
                "input": [{
                    "type": "text",
                    "text": content,
                    "textElements": [],
                }],
            },
        }),
    }
}

fn send_request(
    socket: &mut WebSocket<UnixStream>,
    request: &RpcRequest,
) -> std::result::Result<(), RequestError> {
    socket
        .send(Message::Text(request.payload.to_string().into()))
        .with_context(|| {
            format!(
                "failed to send Codex app-server `{}` request",
                request.method
            )
        })
        .map_err(RequestError::Transport)?;
    wait_for_response(|| socket.read(), request.id, request.method)
}

fn wait_for_response<R>(
    mut read: R,
    request_id: u64,
    method: &str,
) -> std::result::Result<(), RequestError>
where
    R: FnMut() -> tungstenite::Result<Message>,
{
    loop {
        let message = read()
            .with_context(|| format!("failed to read Codex app-server `{method}` response"))
            .map_err(RequestError::Transport)?;
        match message {
            Message::Text(text) => {
                let response: Value = serde_json::from_str(&text)
                    .with_context(|| format!("invalid JSON in `{method}` response"))
                    .map_err(RequestError::Transport)?;
                if is_response_to(&response, request_id, method)? {
                    return Ok(());
                }
            }
            Message::Close(frame) => {
                return Err(RequestError::Transport(anyhow!(
                    "Codex app-server closed during `{method}`: {frame:?}"
                )));
            }
            Message::Binary(_) | Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
        }
    }
}

fn is_response_to(
    response: &Value,
    request_id: u64,
    method: &str,
) -> std::result::Result<bool, RequestError> {
    if response.get("id").and_then(Value::as_u64) != Some(request_id) {
        return Ok(false);
    }
    if let Some(error) = response.get("error") {
        return Err(RequestError::Rejected(anyhow!(
            "Codex app-server `{method}` failed: {}",
            rpc_error(error)
        )));
    }
    if response.get("result").is_none() {
        return Err(RequestError::Transport(anyhow!(
            "Codex app-server `{method}` returned no result"
        )));
    }
    Ok(true)
}

fn rpc_error(error: &Value) -> String {
    let Some(message) = error.get("message").and_then(Value::as_str) else {
        return error.to_string();
    };
    match error.get("code").and_then(Value::as_i64) {
        Some(code) => format!("{message} (code {code})"),
        None => message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[test]
    fn builds_initialize_request() {
        assert_eq!(
            initialize_request().payload,
            json!({
                "id": 1,
                "method": "initialize",
                "params": {
                    "clientInfo": {
                        "name": "armyknife",
                        "title": "armyknife",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                },
            }),
        );
    }

    #[test]
    fn builds_turn_start_request() {
        assert_eq!(
            turn_start_request("thread-a", "hello there").payload,
            json!({
                "id": 2,
                "method": "turn/start",
                "params": {
                    "threadId": "thread-a",
                    "input": [{
                        "type": "text",
                        "text": "hello there",
                        "textElements": [],
                    }],
                },
            }),
        );
    }

    #[rstest]
    #[case::message_and_code(json!({"code": -32600, "message": "Not initialized"}), "Not initialized (code -32600)")]
    #[case::message_only(json!({"message": "thread not found"}), "thread not found")]
    #[case::unknown_shape(json!({"reason": "unavailable"}), r#"{"reason":"unavailable"}"#)]
    fn formats_rpc_errors(#[case] error: Value, #[case] expected: &str) {
        assert_eq!(rpc_error(&error), expected);
    }

    #[rstest]
    #[case::matching_success(json!({"id": 2, "result": {"turn": {"id": "turn-a"}}}), Ok(true))]
    #[case::notification(json!({"method": "turn/started", "params": {}}), Ok(false))]
    #[case::different_request(json!({"id": 1, "result": {}}), Ok(false))]
    #[case::server_error(
        json!({"id": 2, "error": {"code": -32600, "message": "thread not found"}}),
        Err("Codex app-server `turn/start` failed: thread not found (code -32600)")
    )]
    #[case::missing_result(
        json!({"id": 2}),
        Err("Codex app-server `turn/start` returned no result")
    )]
    fn selects_matching_rpc_response(
        #[case] response: Value,
        #[case] expected: std::result::Result<bool, &str>,
    ) {
        assert_eq!(
            is_response_to(&response, 2, "turn/start").map_err(|error| error.to_string()),
            expected.map_err(str::to_string),
        );
    }

    #[test]
    fn waits_through_notifications_for_matching_response() {
        let mut messages = [
            Ok(Message::Text(
                json!({"method": "turn/started", "params": {}})
                    .to_string()
                    .into(),
            )),
            Ok(Message::Text(
                json!({"id": 2, "result": {}}).to_string().into(),
            )),
        ]
        .into_iter();
        let actual = wait_for_response(
            || {
                messages
                    .next()
                    .unwrap_or_else(|| Err(tungstenite::Error::ConnectionClosed))
            },
            2,
            "turn/start",
        );
        assert_eq!(actual.map_err(|error| error.to_string()), Ok(()));
    }

    #[rstest]
    #[case::explicit_codex_home(
        Some("/tmp/codex-home"),
        Some("/tmp/home"),
        Ok(PathBuf::from("/tmp/codex-home/app-server-control/app-server-control.sock"))
    )]
    #[case::default_codex_home(
        None,
        Some("/tmp/home"),
        Ok(PathBuf::from("/tmp/home/.codex/app-server-control/app-server-control.sock"))
    )]
    #[case::empty_codex_home(
        Some(""),
        Some("/tmp/home"),
        Ok(PathBuf::from("/tmp/home/.codex/app-server-control/app-server-control.sock"))
    )]
    #[case::missing_home(
        None,
        None,
        Err("could not determine CODEX_HOME for the app-server control socket")
    )]
    fn resolves_control_socket_path(
        #[case] codex_home: Option<&str>,
        #[case] home: Option<&str>,
        #[case] expected: std::result::Result<PathBuf, &str>,
    ) {
        let actual = temp_env::with_vars(
            [("CODEX_HOME", codex_home), ("HOME", home)],
            control_socket_path,
        );
        assert_eq!(
            actual.map_err(|error| error.to_string()),
            expected.map_err(str::to_string),
        );
    }
}
