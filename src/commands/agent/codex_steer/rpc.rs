use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use serde_json::Value;
use tungstenite::{Message, WebSocket};

use super::{RequestError, RpcRequest};

const RPC_READ_POLL_INTERVAL: Duration = Duration::from_millis(5);

pub(super) fn ignore_interrupted_read(
    result: tungstenite::Result<Message>,
) -> tungstenite::Result<Option<Message>> {
    match result {
        Ok(message) => Ok(Some(message)),
        Err(tungstenite::Error::Io(error)) if error.kind() == std::io::ErrorKind::Interrupted => {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

pub(super) fn send_request(
    socket: &mut WebSocket<UnixStream>,
    request: &RpcRequest,
) -> std::result::Result<(), RequestError> {
    send_request_response(socket, request).map(|_| ())
}

pub(super) fn send_request_response(
    socket: &mut WebSocket<UnixStream>,
    request: &RpcRequest,
) -> std::result::Result<Value, RequestError> {
    send_request_and_collect_notifications(socket, request).map(|(response, _)| response)
}

pub(super) fn send_request_response_until(
    socket: &mut WebSocket<UnixStream>,
    request: &RpcRequest,
    deadline: Instant,
) -> std::result::Result<Value, RequestError> {
    socket
        .send(Message::Text(request.payload.to_string().into()))
        .with_context(|| {
            format!(
                "failed to send Codex app-server `{}` request",
                request.method
            )
        })
        .map_err(RequestError::Transport)?;
    // Per-read timeouts reset for notifications and partial frames.
    // The deadline loop must regain control between reads.
    socket
        .get_ref()
        .set_nonblocking(true)
        .context("failed to enable nonblocking Codex app-server reads")
        .map_err(RequestError::Transport)?;

    let mut remaining = || deadline.saturating_duration_since(Instant::now());
    let result = wait_for_response_with_notifications_until(
        || socket.read(),
        request.id,
        request.method,
        |_| {},
        &mut remaining,
        std::thread::sleep,
    );
    let restore_mode = socket
        .get_ref()
        .set_nonblocking(false)
        .context("failed to restore blocking Codex app-server reads")
        .map_err(RequestError::Transport);

    match (result, restore_mode) {
        (Ok(response), Ok(())) => Ok(response),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(restore_error)) => Err(RequestError::Transport(anyhow!(
            "{error:#}; additionally, {restore_error:#}"
        ))),
    }
}

pub(super) fn send_request_and_collect_notifications(
    socket: &mut WebSocket<UnixStream>,
    request: &RpcRequest,
) -> std::result::Result<(Value, Vec<Value>), RequestError> {
    socket
        .send(Message::Text(request.payload.to_string().into()))
        .with_context(|| {
            format!(
                "failed to send Codex app-server `{}` request",
                request.method
            )
        })
        .map_err(RequestError::Transport)?;
    let mut notifications = Vec::new();
    let response = wait_for_response_with_notifications(
        || socket.read(),
        request.id,
        request.method,
        |notification| notifications.push(notification),
    )?;
    Ok((response, notifications))
}

fn wait_for_response_with_notifications<R, F>(
    mut read: R,
    request_id: u64,
    method: &str,
    mut on_notification: F,
) -> std::result::Result<Value, RequestError>
where
    R: FnMut() -> tungstenite::Result<Message>,
    F: FnMut(Value),
{
    loop {
        let Some(message) = ignore_interrupted_read(read())
            .with_context(|| format!("failed to read Codex app-server `{method}` response"))
            .map_err(RequestError::Transport)?
        else {
            continue;
        };
        if let Some(result) =
            handle_response_message(message, request_id, method, &mut on_notification)?
        {
            return Ok(result);
        }
    }
}

fn wait_for_response_with_notifications_until<R, F, Remaining, Sleep>(
    mut read: R,
    request_id: u64,
    method: &str,
    mut on_notification: F,
    mut remaining: Remaining,
    mut sleep: Sleep,
) -> std::result::Result<Value, RequestError>
where
    R: FnMut() -> tungstenite::Result<Message>,
    F: FnMut(Value),
    Remaining: FnMut() -> Duration,
    Sleep: FnMut(Duration),
{
    loop {
        if remaining().is_zero() {
            return Err(rpc_request_timeout(method));
        }

        match ignore_interrupted_read(read()) {
            Ok(Some(message)) => {
                if remaining().is_zero() {
                    return Err(rpc_request_timeout(method));
                }
                if let Some(response) =
                    handle_response_message(message, request_id, method, &mut on_notification)?
                {
                    if remaining().is_zero() {
                        return Err(rpc_request_timeout(method));
                    }
                    return Ok(response);
                }
            }
            Ok(None) => {}
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                let remaining = remaining();
                if remaining.is_zero() {
                    return Err(rpc_request_timeout(method));
                }
                sleep(RPC_READ_POLL_INTERVAL.min(remaining));
            }
            Err(error) => {
                return Err(RequestError::Transport(anyhow!(error).context(format!(
                    "failed to read Codex app-server `{method}` response"
                ))));
            }
        }
    }
}

fn handle_response_message<F>(
    message: Message,
    request_id: u64,
    method: &str,
    on_notification: &mut F,
) -> std::result::Result<Option<Value>, RequestError>
where
    F: FnMut(Value),
{
    match message {
        Message::Text(text) => {
            let response: Value = serde_json::from_str(&text)
                .with_context(|| format!("invalid JSON in `{method}` response"))
                .map_err(RequestError::Transport)?;
            if let Some(result) = is_response_to(&response, request_id, method)? {
                return Ok(Some(result));
            }
            if response.get("method").and_then(Value::as_str).is_some() {
                on_notification(response);
            }
            Ok(None)
        }
        Message::Close(frame) => Err(RequestError::Transport(anyhow!(
            "Codex app-server closed during `{method}`: {frame:?}"
        ))),
        Message::Binary(_) | Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => Ok(None),
    }
}

fn rpc_request_timeout(method: &str) -> RequestError {
    RequestError::Transport(anyhow!("Codex app-server `{method}` request timed out"))
}

fn is_response_to(
    response: &Value,
    request_id: u64,
    method: &str,
) -> std::result::Result<Option<Value>, RequestError> {
    if response.get("id").and_then(Value::as_u64) != Some(request_id) {
        return Ok(None);
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
    Ok(response.get("result").cloned())
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
    use serde_json::json;
    use tungstenite::protocol::Role;

    #[rstest]
    #[case::message_and_code(json!({"code": -32600, "message": "Not initialized"}), "Not initialized (code -32600)")]
    #[case::message_only(json!({"message": "thread not found"}), "thread not found")]
    #[case::unknown_shape(json!({"reason": "unavailable"}), r#"{"reason":"unavailable"}"#)]
    fn formats_rpc_errors(#[case] error: Value, #[case] expected: &str) {
        assert_eq!(rpc_error(&error), expected);
    }

    #[rstest]
    #[case::matching_success(
        json!({"id": 2, "result": {"turn": {"id": "turn-a"}}}),
        Ok(Some(json!({"turn": {"id": "turn-a"}})))
    )]
    #[case::notification(json!({"method": "turn/started", "params": {}}), Ok(None))]
    #[case::different_request(json!({"id": 1, "result": {}}), Ok(None))]
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
        #[case] expected: std::result::Result<Option<Value>, &str>,
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
        let mut notifications = Vec::new();
        let actual = wait_for_response_with_notifications(
            || {
                messages
                    .next()
                    .unwrap_or_else(|| Err(tungstenite::Error::ConnectionClosed))
            },
            2,
            "turn/start",
            |notification| notifications.push(notification),
        );
        assert_eq!(
            (actual.map_err(|error| error.to_string()), notifications,),
            (
                Ok(json!({})),
                vec![json!({"method": "turn/started", "params": {}})],
            ),
        );
    }

    #[test]
    fn expires_when_notifications_arrive_before_the_same_deadline() {
        let timeout = Duration::from_millis(10);
        let elapsed = std::rc::Rc::new(std::cell::Cell::new(Duration::ZERO));
        let read_elapsed = std::rc::Rc::clone(&elapsed);
        let mut read = move || {
            read_elapsed.set(read_elapsed.get() + Duration::from_millis(4));
            Ok(Message::Text(
                json!({"method": "turn/started", "params": {}})
                    .to_string()
                    .into(),
            ))
        };
        let deadline_elapsed = std::rc::Rc::clone(&elapsed);
        let mut remaining = move || timeout.saturating_sub(deadline_elapsed.get());
        let mut notifications = Vec::new();

        let actual = wait_for_response_with_notifications_until(
            &mut read,
            2,
            "thread/read",
            |notification| notifications.push(notification),
            &mut remaining,
            |_| {},
        );

        assert_eq!(
            (
                actual.map_err(|error| error.to_string()),
                notifications,
                elapsed.get(),
            ),
            (
                Err("Codex app-server `thread/read` request timed out".to_string()),
                vec![
                    json!({"method": "turn/started", "params": {}}),
                    json!({"method": "turn/started", "params": {}}),
                ],
                Duration::from_millis(12),
            ),
        );
    }

    #[test]
    fn expires_while_waiting_for_more_bytes_of_a_partial_frame() {
        let timeout = Duration::from_millis(10);
        let elapsed = std::rc::Rc::new(std::cell::Cell::new(Duration::ZERO));
        let read_count = std::rc::Rc::new(std::cell::Cell::new(0));
        let read_count_for_read = std::rc::Rc::clone(&read_count);
        let mut read = move || {
            read_count_for_read.set(read_count_for_read.get() + 1);
            Err(tungstenite::Error::Io(std::io::Error::from(
                std::io::ErrorKind::WouldBlock,
            )))
        };
        let deadline_elapsed = std::rc::Rc::clone(&elapsed);
        let mut remaining = move || timeout.saturating_sub(deadline_elapsed.get());
        let sleep_elapsed = std::rc::Rc::clone(&elapsed);

        let actual = wait_for_response_with_notifications_until(
            &mut read,
            2,
            "thread/read",
            |_| {},
            &mut remaining,
            move |duration| sleep_elapsed.set(sleep_elapsed.get() + duration),
        );

        assert_eq!(
            (
                actual.map_err(|error| error.to_string()),
                read_count.get(),
                elapsed.get(),
            ),
            (
                Err("Codex app-server `thread/read` request timed out".to_string()),
                2,
                timeout,
            ),
        );
    }

    #[test]
    fn resumes_a_nonblocking_read_after_a_partial_websocket_frame() -> anyhow::Result<()> {
        use std::io::Write;

        let (client_stream, mut server_stream) = UnixStream::pair()?;
        let mut socket = WebSocket::from_raw_socket(client_stream, Role::Client, None);
        socket.get_ref().set_nonblocking(true)?;

        let text = json!({"id": 2, "result": {"state": "complete"}}).to_string();
        let payload = text.as_bytes();
        let frame_header = [0x81, u8::try_from(payload.len())?];
        let split = payload.len() / 2;
        server_stream.write_all(&frame_header)?;
        server_stream.write_all(&payload[..split])?;
        let partial_read = socket.read().map_err(|error| match error {
            tungstenite::Error::Io(error) => error.kind(),
            _ => std::io::ErrorKind::Other,
        });

        server_stream.write_all(&payload[split..])?;
        let complete_read = socket.read().map_err(|error| match error {
            tungstenite::Error::Io(error) => error.kind(),
            _ => std::io::ErrorKind::Other,
        });

        assert_eq!(
            (partial_read, complete_read),
            (
                Err(std::io::ErrorKind::WouldBlock),
                Ok(Message::Text(text.into())),
            ),
        );
        Ok(())
    }

    #[test]
    fn waits_for_matching_response_after_interrupted_read() {
        let mut messages = [
            Err(tungstenite::Error::Io(std::io::Error::from(
                std::io::ErrorKind::Interrupted,
            ))),
            Ok(Message::Text(
                json!({"id": 2, "result": {"turn": {"id": "turn-a"}}})
                    .to_string()
                    .into(),
            )),
        ]
        .into_iter();

        let actual = wait_for_response_with_notifications(
            || {
                messages
                    .next()
                    .unwrap_or_else(|| Err(tungstenite::Error::ConnectionClosed))
            },
            2,
            "turn/start",
            |_| {},
        );

        assert_eq!(
            actual.map_err(|error| error.to_string()),
            Ok(json!({"turn": {"id": "turn-a"}}))
        );
    }
}
