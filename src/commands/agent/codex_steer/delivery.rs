use std::io::ErrorKind;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use anyhow::{Context, anyhow};
use serde_json::{Value, json};
use tungstenite::{Message, WebSocket};

use crate::commands::agent::types::ReasoningEffort;

use super::{DeliveryError, RequestError, RpcRequest};
mod history;
use history::{
    MatchingItemSnapshot, has_completed_turn_notification, has_recorded_message,
    has_user_message_notification, is_terminal_turn_status, latest_turn_status as latest_status,
    matching_user_message_snapshot, turn_status,
};

const PRE_TURN_READ_REQUEST_ID: u64 = 2;
pub(super) const THREAD_RESUME_REQUEST_ID: u64 = 4;
const THREAD_READ_REQUEST_ID: u64 = 5;
const NEXT_THREAD_READ_REQUEST_ID: u64 = 6;
const RECOVERY_THREAD_RESUME_REQUEST_ID: u64 = 7;
const RETRY_TURN_START_REQUEST_ID: u64 = RECOVERY_THREAD_RESUME_REQUEST_ID + 1;
const DELIVERY_POLL_INTERVAL: Duration = Duration::from_secs(5);

struct DeliveryContext<'a> {
    thread_id: &'a str,
    turn_id: &'a str,
    client_user_message_id: &'a str,
    content: &'a str,
    snapshot: &'a MatchingItemSnapshot,
}

pub(super) fn send_and_confirm(
    socket: &mut WebSocket<UnixStream>,
    thread_id: &str,
    content: &str,
    effort: Option<ReasoningEffort>,
    client_user_message_id: &str,
) -> super::Result<()> {
    send_and_confirm_with_reader(
        socket,
        thread_id,
        content,
        effort,
        client_user_message_id,
        |socket| socket.read(),
    )
}

fn send_and_confirm_with_reader<R>(
    socket: &mut WebSocket<UnixStream>,
    thread_id: &str,
    content: &str,
    effort: Option<ReasoningEffort>,
    client_user_message_id: &str,
    mut read: R,
) -> super::Result<()>
where
    R: FnMut(&mut WebSocket<UnixStream>) -> tungstenite::Result<Message>,
{
    // History is only used as a comparison baseline; a rejected read must not block delivery.
    let snapshot = match read_thread(socket, thread_id, PRE_TURN_READ_REQUEST_ID) {
        Ok((response, _)) => {
            let thread = response.get("thread").cloned().ok_or_else(|| {
                DeliveryError::NotDelivered(anyhow!(
                    "Codex app-server thread/read response did not contain a thread before turn/start"
                ))
            })?;
            matching_user_message_snapshot(&thread, content)
        }
        Err(RequestError::Rejected(error)) => {
            tracing::warn!(
                event = "agent.codex_steer.pre_turn_read_rejected",
                session = %thread_id,
                error = %error,
            );
            MatchingItemSnapshot::default()
        }
        Err(error) => {
            return Err(DeliveryError::NotDelivered(
                super::request_error_to_anyhow(error)
                    .context("could not read Codex thread before starting the turn"),
            ));
        }
    };

    let (turn_start_response, next_thread_read_request_id) =
        start_turn_with_recovery(socket, thread_id, content, effort, client_user_message_id)?;
    let turn_id = turn_start_response
        .pointer("/turn/id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            DeliveryError::Unconfirmed(anyhow!(
                "Codex app-server turn/start response did not contain a turn ID"
            ))
        })?;
    let delivery = DeliveryContext {
        thread_id,
        turn_id,
        client_user_message_id,
        content,
        snapshot: &snapshot,
    };

    // `turn/start` only acknowledges queueing, so resume to observe the userMessage item event.
    let (resume_response, resume_notifications) =
        super::send_request_and_collect_notifications(socket, &thread_resume_request(thread_id))
            .map_err(|error| uncertain_request_error(error, "thread/resume"))?;
    let resumed_thread = resume_response.get("thread").ok_or_else(|| {
        DeliveryError::Unconfirmed(anyhow!(
            "Codex app-server thread/resume response did not contain a thread"
        ))
    })?;
    if has_recorded_message(
        resumed_thread,
        &resume_notifications,
        turn_id,
        client_user_message_id,
        content,
        &snapshot,
    ) {
        return Ok(());
    }

    let (read_response, read_notifications) =
        read_thread(socket, thread_id, THREAD_READ_REQUEST_ID)
            .map_err(|error| uncertain_request_error(error, "thread/read"))?;
    let thread = read_response.get("thread").ok_or_else(|| {
        DeliveryError::Unconfirmed(anyhow!(
            "Codex app-server thread/read response did not contain a thread"
        ))
    })?;
    if has_recorded_message(
        thread,
        &read_notifications,
        turn_id,
        client_user_message_id,
        content,
        &snapshot,
    ) {
        return Ok(());
    }

    if has_completed_turn_notification(&resume_notifications, turn_id)
        || has_completed_turn_notification(&read_notifications, turn_id)
    {
        let mut next_read_request_id = next_thread_read_request_id;
        return confirm_after_turn_end(socket, &delivery, &mut next_read_request_id);
    }

    let mut next_read_request_id = next_thread_read_request_id;
    match turn_status(thread, turn_id) {
        Some(status) if is_terminal_turn_status(status) => {
            confirm_after_turn_end(socket, &delivery, &mut next_read_request_id)
        }
        Some("inProgress") => {
            wait_for_delivery_with_reader(socket, &delivery, &mut next_read_request_id, &mut read)
        }
        Some(status) => Err(DeliveryError::Unconfirmed(anyhow!(
            "Codex app-server returned unknown turn status {status}"
        ))),
        None => {
            wait_for_delivery_with_reader(socket, &delivery, &mut next_read_request_id, &mut read)
        }
    }
}

fn start_turn_with_recovery(
    socket: &mut WebSocket<UnixStream>,
    thread_id: &str,
    content: &str,
    effort: Option<ReasoningEffort>,
    client_user_message_id: &str,
) -> super::Result<(Value, u64)> {
    let initial_request =
        super::turn_start_request(thread_id, content, effort, client_user_message_id);
    match super::send_request_response(socket, &initial_request) {
        Ok(response) => Ok((response, NEXT_THREAD_READ_REQUEST_ID)),
        Err(error) if is_unloaded_thread_rejection(&error, thread_id) => {
            let recovery =
                thread_resume_request_with_id(thread_id, RECOVERY_THREAD_RESUME_REQUEST_ID);
            super::send_request_response(socket, &recovery).map_err(|error| {
                DeliveryError::NotDelivered(
                    super::request_error_to_anyhow(error)
                        .context("could not resume Codex thread after it was not loaded"),
                )
            })?;

            let retry = super::turn_start_request_with_id(
                RETRY_TURN_START_REQUEST_ID,
                thread_id,
                content,
                effort,
                client_user_message_id,
            );
            let next_read_request_id =
                RETRY_TURN_START_REQUEST_ID.checked_add(1).ok_or_else(|| {
                    DeliveryError::Unconfirmed(anyhow!(
                        "Codex app-server request ID space was exhausted after retrying turn/start"
                    ))
                })?;
            let response =
                super::send_request_response(socket, &retry).map_err(start_request_error)?;
            Ok((response, next_read_request_id))
        }
        Err(error) => Err(start_request_error(error)),
    }
}

fn is_unloaded_thread_rejection(error: &RequestError, thread_id: &str) -> bool {
    let expected_message = format!("thread not found: {thread_id}");
    matches!(
        error,
        RequestError::Rejected(rejection)
            if rejection.code == Some(-32600)
                && rejection.message.as_deref() == Some(expected_message.as_str())
    )
}

fn wait_for_delivery_with_reader<R>(
    socket: &mut WebSocket<UnixStream>,
    delivery: &DeliveryContext<'_>,
    next_read_request_id: &mut u64,
    mut read: R,
) -> super::Result<()>
where
    R: FnMut(&mut WebSocket<UnixStream>) -> tungstenite::Result<Message>,
{
    loop {
        socket
            .get_ref()
            .set_read_timeout(Some(DELIVERY_POLL_INTERVAL))
            .context("failed to set Codex app-server delivery notification timeout")
            .map_err(DeliveryError::Unconfirmed)?;
        let message = match super::ignore_interrupted_read(read(socket)) {
            Ok(Some(message)) => message,
            Ok(None) => continue,
            Err(error) if is_read_timeout(&error) => {
                let request_id = take_request_id(next_read_request_id)?;
                let (response, notifications) = read_thread(socket, delivery.thread_id, request_id)
                    .map_err(|error| uncertain_request_error(error, "thread/read"))?;
                let thread = response.get("thread").ok_or_else(|| {
                    DeliveryError::Unconfirmed(anyhow!(
                        "Codex app-server thread/read response did not contain a thread"
                    ))
                })?;
                if has_recorded_message(
                    thread,
                    &notifications,
                    delivery.turn_id,
                    delivery.client_user_message_id,
                    delivery.content,
                    delivery.snapshot,
                ) {
                    return Ok(());
                }
                if has_completed_turn_notification(&notifications, delivery.turn_id) {
                    return confirm_after_turn_end(socket, delivery, next_read_request_id);
                }
                match turn_status(thread, delivery.turn_id) {
                    Some(status) if is_terminal_turn_status(status) => {
                        return confirm_after_turn_end(socket, delivery, next_read_request_id);
                    }
                    Some("inProgress") => continue,
                    Some(status) => {
                        return Err(DeliveryError::Unconfirmed(anyhow!(
                            "Codex app-server returned unknown turn status {status}"
                        )));
                    }
                    None => continue,
                }
            }
            Err(error) => {
                return Err(DeliveryError::Unconfirmed(
                    anyhow!(error)
                        .context("failed to read Codex app-server delivery notifications"),
                ));
            }
        };
        match message {
            Message::Text(text) => {
                let notification: Value = serde_json::from_str(&text).map_err(|error| {
                    DeliveryError::Unconfirmed(
                        anyhow!(error)
                            .context("invalid JSON in Codex app-server delivery notification"),
                    )
                })?;
                if has_user_message_notification(
                    std::slice::from_ref(&notification),
                    delivery.turn_id,
                    delivery.client_user_message_id,
                    delivery.content,
                    delivery.snapshot,
                ) {
                    return Ok(());
                }
                if has_completed_turn_notification(
                    std::slice::from_ref(&notification),
                    delivery.turn_id,
                ) {
                    return confirm_after_turn_end(socket, delivery, next_read_request_id);
                }
            }
            Message::Close(frame) => {
                return Err(DeliveryError::Unconfirmed(anyhow!(
                    "Codex app-server closed before delivery was confirmed: {frame:?}"
                )));
            }
            Message::Binary(_) | Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
        }
    }
}

fn confirm_after_turn_end(
    socket: &mut WebSocket<UnixStream>,
    delivery: &DeliveryContext<'_>,
    next_read_request_id: &mut u64,
) -> super::Result<()> {
    let request_id = take_request_id(next_read_request_id)?;
    let (response, notifications) = read_thread(socket, delivery.thread_id, request_id)
        .map_err(|error| uncertain_request_error(error, "thread/read"))?;
    let thread = response.get("thread").ok_or_else(|| {
        DeliveryError::Unconfirmed(anyhow!(
            "Codex app-server thread/read response did not contain a thread"
        ))
    })?;
    if has_recorded_message(
        thread,
        &notifications,
        delivery.turn_id,
        delivery.client_user_message_id,
        delivery.content,
        delivery.snapshot,
    ) {
        return Ok(());
    }
    if delivery.snapshot.baseline_available() {
        Err(not_delivered())
    } else {
        Err(DeliveryError::Unconfirmed(anyhow!(
            "Codex app-server turn ended without a matching client user message ID in thread history"
        )))
    }
}

fn read_thread(
    socket: &mut WebSocket<UnixStream>,
    thread_id: &str,
    request_id: u64,
) -> std::result::Result<(Value, Vec<Value>), RequestError> {
    socket
        .get_ref()
        .set_read_timeout(Some(super::RESPONSE_TIMEOUT))
        .context("failed to set Codex app-server thread/read response timeout")
        .map_err(RequestError::Transport)?;
    super::send_request_and_collect_notifications(
        socket,
        &thread_read_request(thread_id, request_id),
    )
}

pub(super) fn latest_turn_status(
    socket: &mut WebSocket<UnixStream>,
    thread_id: &str,
    request_id: u64,
) -> anyhow::Result<Option<String>> {
    let (response, _) =
        read_thread(socket, thread_id, request_id).map_err(super::request_error_to_anyhow)?;
    let thread = response
        .get("thread")
        .ok_or_else(|| anyhow!("Codex app-server thread/read response did not contain a thread"))?;
    Ok(latest_status(thread).map(str::to_string))
}

fn take_request_id(next_request_id: &mut u64) -> super::Result<u64> {
    let request_id = *next_request_id;
    *next_request_id = next_request_id.checked_add(1).ok_or_else(|| {
        DeliveryError::Unconfirmed(anyhow!(
            "Codex app-server request ID space was exhausted while checking delivery"
        ))
    })?;
    Ok(request_id)
}

fn thread_resume_request(thread_id: &str) -> RpcRequest {
    thread_resume_request_with_id(thread_id, THREAD_RESUME_REQUEST_ID)
}

fn thread_resume_request_with_id(thread_id: &str, request_id: u64) -> RpcRequest {
    RpcRequest {
        id: request_id,
        method: "thread/resume",
        payload: json!({
            "id": request_id,
            "method": "thread/resume",
            "params": {"threadId": thread_id},
        }),
    }
}

fn thread_read_request(thread_id: &str, request_id: u64) -> RpcRequest {
    RpcRequest {
        id: request_id,
        method: "thread/read",
        payload: json!({
            "id": request_id,
            "method": "thread/read",
            "params": {
                "threadId": thread_id,
                "includeTurns": true,
            },
        }),
    }
}

fn is_read_timeout(error: &tungstenite::Error) -> bool {
    matches!(
        error,
        tungstenite::Error::Io(error)
            if matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock)
    )
}

fn not_delivered() -> DeliveryError {
    DeliveryError::NotDelivered(anyhow!(
        "Codex app-server turn ended before a matching user message appeared in thread history"
    ))
}

fn start_request_error(error: RequestError) -> DeliveryError {
    match error {
        RequestError::Rejected(error) => DeliveryError::NotDelivered(anyhow::Error::new(error)),
        RequestError::Transport(error) => DeliveryError::Unconfirmed(error),
    }
}

fn uncertain_request_error(error: RequestError, method: &str) -> DeliveryError {
    DeliveryError::Unconfirmed(
        super::request_error_to_anyhow(error)
            .context(format!("could not confirm Codex delivery using {method}")),
    )
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;

    use rstest::{fixture, rstest};
    use serde_json::json;
    use tungstenite::{Message, WebSocket, client};

    use super::super::RpcRejection;
    use super::super::TURN_START_REQUEST_ID;
    use super::super::test_support::{accept_with_read_timeout, join_server};
    use super::*;

    const THREAD_ID: &str = "thread-a";
    const TURN_ID: &str = "turn-a";
    const CLIENT_USER_MESSAGE_ID: &str = "message-a";

    #[fixture]
    fn connected_sockets() -> anyhow::Result<(WebSocket<UnixStream>, WebSocket<UnixStream>)> {
        let (client_stream, server_stream) = UnixStream::pair()?;
        let server = std::thread::spawn(move || accept_with_read_timeout(server_stream));
        let (client_socket, _) = client("ws://localhost/rpc", client_stream)?;
        let server_socket = server
            .join()
            .map_err(|_| anyhow!("Codex app-server test thread panicked"))??;

        Ok((client_socket, server_socket))
    }

    fn read_request(socket: &mut WebSocket<UnixStream>) -> anyhow::Result<Value> {
        let Message::Text(request) = socket.read()? else {
            return Err(anyhow!("Codex app-server received a non-text request"));
        };
        Ok(serde_json::from_str(&request)?)
    }

    fn send_response(
        socket: &mut WebSocket<UnixStream>,
        request_id: u64,
        result: Value,
    ) -> anyhow::Result<()> {
        socket.send(Message::Text(
            json!({"id": request_id, "result": result})
                .to_string()
                .into(),
        ))?;
        Ok(())
    }

    fn send_error(
        socket: &mut WebSocket<UnixStream>,
        request_id: u64,
        code: i64,
        message: &str,
    ) -> anyhow::Result<()> {
        socket.send(Message::Text(
            json!({
                "id": request_id,
                "error": {"code": code, "message": message},
            })
            .to_string()
            .into(),
        ))?;
        Ok(())
    }

    fn thread_response(
        status: &str,
        client_user_message_id: Option<&str>,
        content: Option<&str>,
    ) -> Value {
        let items = if client_user_message_id.is_some() || content.is_some() {
            let mut item = json!({
                "type": "userMessage",
                "id": "item-a",
                "content": content
                    .map(|text| vec![json!({"type": "text", "text": text})])
                    .unwrap_or_default(),
            });
            if let Some(client_id) = client_user_message_id {
                item["clientId"] = json!(client_id);
            }
            vec![item]
        } else {
            Vec::new()
        };
        json!({
            "thread": {"turns": [{
                "id": TURN_ID,
                "status": status,
                "items": items,
            }]},
        })
    }

    fn start_and_subscribe_requests() -> Vec<Value> {
        vec![
            thread_read_request(PRE_TURN_READ_REQUEST_ID),
            json!({
                "id": TURN_START_REQUEST_ID,
                "method": "turn/start",
                "params": {
                    "threadId": THREAD_ID,
                    "clientUserMessageId": CLIENT_USER_MESSAGE_ID,
                    "input": [{
                        "type": "text",
                        "text": "hello there",
                        "textElements": [],
                    }],
                },
            }),
            json!({
                "id": THREAD_RESUME_REQUEST_ID,
                "method": "thread/resume",
                "params": {"threadId": THREAD_ID},
            }),
        ]
    }

    fn thread_read_request(request_id: u64) -> Value {
        json!({
            "id": request_id,
            "method": "thread/read",
            "params": {
                "threadId": THREAD_ID,
                "includeTurns": true,
            },
        })
    }

    fn read_snapshot(socket: &mut WebSocket<UnixStream>, result: Value) -> anyhow::Result<Value> {
        let request = read_request(socket)?;
        send_response(socket, PRE_TURN_READ_REQUEST_ID, result)?;
        Ok(request)
    }

    fn read_empty_snapshot(socket: &mut WebSocket<UnixStream>) -> anyhow::Result<Value> {
        read_snapshot(socket, json!({"thread": {"turns": []}}))
    }

    fn user_message_notification() -> Value {
        json!({
            "method": "item/started",
            "params": {
                "threadId": THREAD_ID,
                "turnId": TURN_ID,
                "item": {
                    "type": "userMessage",
                    "id": "item-a",
                    "content": [{"type": "text", "text": "hello there"}],
                },
            },
        })
    }

    fn result_text(result: super::super::Result<()>) -> std::result::Result<(), String> {
        result.map_err(|error| error.to_string())
    }

    fn result_classification(result: super::super::Result<()>) -> (&'static str, String) {
        match result {
            Ok(()) => ("delivered", String::new()),
            Err(error @ DeliveryError::NotDelivered(_)) => ("not_delivered", error.to_string()),
            Err(error @ DeliveryError::Unconfirmed(_)) => ("unconfirmed", error.to_string()),
        }
    }

    #[test]
    fn confirms_delivery_when_matching_user_message_starts() -> anyhow::Result<()> {
        let (mut client_socket, server_socket) = connected_sockets()?;
        let (notification_read_tx, notification_read_rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || -> anyhow::Result<Vec<Value>> {
            let mut socket = server_socket;
            let snapshot_request = read_empty_snapshot(&mut socket)?;
            let turn_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                TURN_START_REQUEST_ID,
                json!({"turn": {"id": TURN_ID, "status": "inProgress"}}),
            )?;
            let resume_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                THREAD_RESUME_REQUEST_ID,
                json!({"thread": {"turns": []}}),
            )?;
            let history_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                THREAD_READ_REQUEST_ID,
                json!({"thread": {"turns": []}}),
            )?;
            socket.send(Message::Text(
                user_message_notification().to_string().into(),
            ))?;
            notification_read_rx.recv_timeout(Duration::from_secs(5))?;
            Ok(vec![
                snapshot_request,
                turn_request,
                resume_request,
                history_request,
            ])
        });

        let result = result_text(send_and_confirm_with_reader(
            &mut client_socket,
            THREAD_ID,
            "hello there",
            None,
            CLIENT_USER_MESSAGE_ID,
            |socket| {
                let message = socket.read();
                if matches!(&message, Ok(Message::Text(_))) {
                    let _ = notification_read_tx.send(());
                }
                message
            },
        ));
        let requests = join_server(&client_socket, &result, server)?;
        let expected_requests = start_and_subscribe_requests()
            .into_iter()
            .chain(std::iter::once(thread_read_request(THREAD_READ_REQUEST_ID)))
            .collect::<Vec<_>>();

        assert_eq!((result, requests), (Ok(()), expected_requests));
        Ok(())
    }

    #[rstest]
    fn confirms_delivery_after_interrupted_notification_read(
        connected_sockets: anyhow::Result<(WebSocket<UnixStream>, WebSocket<UnixStream>)>,
    ) -> anyhow::Result<()> {
        let (mut client_socket, _server_socket) = connected_sockets?;
        let snapshot = matching_user_message_snapshot(&json!({"turns": []}), "hello there");
        let delivery = DeliveryContext {
            thread_id: THREAD_ID,
            turn_id: TURN_ID,
            client_user_message_id: CLIENT_USER_MESSAGE_ID,
            content: "hello there",
            snapshot: &snapshot,
        };
        let mut messages = [
            Err(tungstenite::Error::Io(std::io::Error::from(
                std::io::ErrorKind::Interrupted,
            ))),
            Ok(Message::Text(
                user_message_notification().to_string().into(),
            )),
        ]
        .into_iter();
        let mut next_read_request_id = NEXT_THREAD_READ_REQUEST_ID;

        let actual = wait_for_delivery_with_reader(
            &mut client_socket,
            &delivery,
            &mut next_read_request_id,
            |_| {
                messages
                    .next()
                    .unwrap_or_else(|| Err(tungstenite::Error::ConnectionClosed))
            },
        );

        assert_eq!(result_text(actual), Ok(()));
        Ok(())
    }

    #[rstest]
    #[case::paginated(json!({"code": -32601, "message": "list_turns is not supported yet"}))]
    #[case::legacy(json!({
        "code": -32600,
        "message": "thread thread-a is not materialized yet; includeTurns is unavailable before first user message",
    }))]
    #[case::changed_message(json!({
        "code": -32000,
        "message": "thread history is temporarily unavailable",
    }))]
    fn proceeds_when_pre_turn_read_is_rejected(
        connected_sockets: anyhow::Result<(WebSocket<UnixStream>, WebSocket<UnixStream>)>,
        #[case] error: Value,
    ) -> anyhow::Result<()> {
        let (mut client_socket, server_socket) = connected_sockets?;
        let server = std::thread::spawn(move || -> anyhow::Result<Vec<Value>> {
            let mut socket = server_socket;
            let snapshot_request = read_request(&mut socket)?;
            socket.send(Message::Text(
                json!({"id": PRE_TURN_READ_REQUEST_ID, "error": error})
                    .to_string()
                    .into(),
            ))?;
            let turn_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                TURN_START_REQUEST_ID,
                json!({"turn": {"id": TURN_ID, "status": "inProgress"}}),
            )?;
            let resume_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                THREAD_RESUME_REQUEST_ID,
                thread_response(
                    "inProgress",
                    Some(CLIENT_USER_MESSAGE_ID),
                    Some("hello there"),
                ),
            )?;
            Ok(vec![snapshot_request, turn_request, resume_request])
        });

        let result = result_text(send_and_confirm(
            &mut client_socket,
            THREAD_ID,
            "hello there",
            None,
            CLIENT_USER_MESSAGE_ID,
        ));
        let requests = join_server(&client_socket, &result, server)?;

        assert_eq!((result, requests), (Ok(()), start_and_subscribe_requests()));
        Ok(())
    }

    #[rstest]
    #[case::rejected_snapshot(-32000, "thread history is temporarily unavailable")]
    fn does_not_confirm_same_body_notification_after_rejected_pre_turn_read(
        connected_sockets: anyhow::Result<(WebSocket<UnixStream>, WebSocket<UnixStream>)>,
        #[case] rejection_code: i64,
        #[case] rejection_message: &'static str,
    ) -> anyhow::Result<()> {
        let (mut client_socket, server_socket) = connected_sockets?;
        let server = std::thread::spawn(move || -> anyhow::Result<Vec<Value>> {
            let mut socket = server_socket;
            let snapshot_request = read_request(&mut socket)?;
            send_error(
                &mut socket,
                PRE_TURN_READ_REQUEST_ID,
                rejection_code,
                rejection_message,
            )?;
            let turn_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                TURN_START_REQUEST_ID,
                json!({"turn": {"id": TURN_ID, "status": "inProgress"}}),
            )?;
            let resume_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                THREAD_RESUME_REQUEST_ID,
                thread_response("inProgress", None, None),
            )?;
            let history_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                THREAD_READ_REQUEST_ID,
                thread_response("inProgress", None, None),
            )?;
            let final_history_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                NEXT_THREAD_READ_REQUEST_ID,
                thread_response("interrupted", None, Some("hello there")),
            )?;
            Ok(vec![
                snapshot_request,
                turn_request,
                resume_request,
                history_request,
                final_history_request,
            ])
        });
        let mut notifications = [
            Ok(Message::Text(
                user_message_notification().to_string().into(),
            )),
            Ok(Message::Text(
                json!({
                    "method": "turn/completed",
                    "params": {
                        "threadId": THREAD_ID,
                        "turn": {"id": TURN_ID, "status": "interrupted", "items": []},
                    },
                })
                .to_string()
                .into(),
            )),
        ]
        .into_iter();

        let result = result_text(send_and_confirm_with_reader(
            &mut client_socket,
            THREAD_ID,
            "hello there",
            None,
            CLIENT_USER_MESSAGE_ID,
            |_| {
                notifications
                    .next()
                    .unwrap_or(Err(tungstenite::Error::ConnectionClosed))
            },
        ));
        let requests = join_server(&client_socket, &result, server)?;
        let expected_requests = start_and_subscribe_requests()
            .into_iter()
            .chain([
                thread_read_request(THREAD_READ_REQUEST_ID),
                thread_read_request(NEXT_THREAD_READ_REQUEST_ID),
            ])
            .collect::<Vec<_>>();

        assert_eq!(
            (result, requests),
            (
                Err("Codex app-server delivery is unconfirmed: Codex app-server turn ended without a matching client user message ID in thread history".to_string()),
                expected_requests,
            ),
        );
        Ok(())
    }

    #[rstest]
    #[case::exact_rejection(-32600, "thread not found: thread-a", true)]
    #[case::wrong_code(-32601, "thread not found: thread-a", false)]
    #[case::wrong_thread(-32600, "thread not found: thread-b", false)]
    #[case::extra_message_text(-32600, "thread not found: thread-a; retry", false)]
    fn recognizes_only_exact_unloaded_thread_rejection(
        #[case] code: i64,
        #[case] message: &str,
        #[case] expected: bool,
    ) {
        let error = RequestError::Rejected(RpcRejection::new(
            "turn/start",
            &json!({"code": code, "message": message}),
        ));

        assert_eq!(is_unloaded_thread_rejection(&error, THREAD_ID), expected);
    }

    #[rstest]
    fn resumes_unloaded_thread_and_retries_once_with_the_same_message_id(
        connected_sockets: anyhow::Result<(WebSocket<UnixStream>, WebSocket<UnixStream>)>,
    ) -> anyhow::Result<()> {
        let (mut client_socket, server_socket) = connected_sockets?;
        let server = std::thread::spawn(move || -> anyhow::Result<Vec<Value>> {
            let mut socket = server_socket;
            let snapshot_request = read_empty_snapshot(&mut socket)?;
            let first_turn_request = read_request(&mut socket)?;
            send_error(
                &mut socket,
                TURN_START_REQUEST_ID,
                -32600,
                "thread not found: thread-a",
            )?;
            let recovery_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                RECOVERY_THREAD_RESUME_REQUEST_ID,
                json!({"thread": {"turns": []}}),
            )?;
            let retry_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                RETRY_TURN_START_REQUEST_ID,
                json!({"turn": {"id": TURN_ID, "status": "inProgress"}}),
            )?;
            let turn_resume_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                THREAD_RESUME_REQUEST_ID,
                thread_response(
                    "inProgress",
                    Some(CLIENT_USER_MESSAGE_ID),
                    Some("hello there"),
                ),
            )?;
            Ok(vec![
                snapshot_request,
                first_turn_request,
                recovery_request,
                retry_request,
                turn_resume_request,
            ])
        });

        let result = result_text(send_and_confirm(
            &mut client_socket,
            THREAD_ID,
            "hello there",
            None,
            CLIENT_USER_MESSAGE_ID,
        ));
        let requests = join_server(&client_socket, &result, server)?;
        let expected_requests = vec![
            thread_read_request(PRE_TURN_READ_REQUEST_ID),
            super::super::turn_start_request(
                THREAD_ID,
                "hello there",
                None,
                CLIENT_USER_MESSAGE_ID,
            )
            .payload,
            thread_resume_request_with_id(THREAD_ID, RECOVERY_THREAD_RESUME_REQUEST_ID).payload,
            super::super::turn_start_request_with_id(
                RETRY_TURN_START_REQUEST_ID,
                THREAD_ID,
                "hello there",
                None,
                CLIENT_USER_MESSAGE_ID,
            )
            .payload,
            thread_resume_request(THREAD_ID).payload,
        ];

        assert_eq!((result, requests), (Ok(()), expected_requests));
        Ok(())
    }

    #[rstest]
    fn confirms_recovered_delivery_from_a_later_history_read(
        connected_sockets: anyhow::Result<(WebSocket<UnixStream>, WebSocket<UnixStream>)>,
    ) -> anyhow::Result<()> {
        let (mut client_socket, server_socket) = connected_sockets?;
        let server = std::thread::spawn(move || -> anyhow::Result<Vec<Value>> {
            let mut socket = server_socket;
            let snapshot_request = read_empty_snapshot(&mut socket)?;
            let first_turn_request = read_request(&mut socket)?;
            send_error(
                &mut socket,
                TURN_START_REQUEST_ID,
                -32600,
                "thread not found: thread-a",
            )?;
            let recovery_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                RECOVERY_THREAD_RESUME_REQUEST_ID,
                json!({"thread": {"turns": []}}),
            )?;
            let retry_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                RETRY_TURN_START_REQUEST_ID,
                json!({"turn": {"id": TURN_ID, "status": "inProgress"}}),
            )?;
            let turn_resume_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                THREAD_RESUME_REQUEST_ID,
                thread_response("inProgress", None, None),
            )?;
            let initial_history_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                THREAD_READ_REQUEST_ID,
                thread_response("inProgress", None, None),
            )?;
            let delayed_history_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                RETRY_TURN_START_REQUEST_ID + 1,
                thread_response(
                    "inProgress",
                    Some(CLIENT_USER_MESSAGE_ID),
                    Some("hello there"),
                ),
            )?;
            Ok(vec![
                snapshot_request,
                first_turn_request,
                recovery_request,
                retry_request,
                turn_resume_request,
                initial_history_request,
                delayed_history_request,
            ])
        });

        let result = result_text(send_and_confirm_with_reader(
            &mut client_socket,
            THREAD_ID,
            "hello there",
            None,
            CLIENT_USER_MESSAGE_ID,
            |_| {
                Err(tungstenite::Error::Io(std::io::Error::from(
                    ErrorKind::WouldBlock,
                )))
            },
        ));
        let requests = join_server(&client_socket, &result, server)?;
        let expected_requests = vec![
            thread_read_request(PRE_TURN_READ_REQUEST_ID),
            super::super::turn_start_request(
                THREAD_ID,
                "hello there",
                None,
                CLIENT_USER_MESSAGE_ID,
            )
            .payload,
            thread_resume_request_with_id(THREAD_ID, RECOVERY_THREAD_RESUME_REQUEST_ID).payload,
            super::super::turn_start_request_with_id(
                RETRY_TURN_START_REQUEST_ID,
                THREAD_ID,
                "hello there",
                None,
                CLIENT_USER_MESSAGE_ID,
            )
            .payload,
            thread_resume_request(THREAD_ID).payload,
            thread_read_request(THREAD_READ_REQUEST_ID),
            thread_read_request(RETRY_TURN_START_REQUEST_ID + 1),
        ];

        assert_eq!((result, requests), (Ok(()), expected_requests));
        Ok(())
    }

    #[rstest]
    fn does_not_retry_when_unloaded_thread_cannot_be_resumed(
        connected_sockets: anyhow::Result<(WebSocket<UnixStream>, WebSocket<UnixStream>)>,
    ) -> anyhow::Result<()> {
        let (mut client_socket, server_socket) = connected_sockets?;
        let server = std::thread::spawn(move || -> anyhow::Result<Vec<Value>> {
            let mut socket = server_socket;
            let snapshot_request = read_empty_snapshot(&mut socket)?;
            let turn_request = read_request(&mut socket)?;
            send_error(
                &mut socket,
                TURN_START_REQUEST_ID,
                -32600,
                "thread not found: thread-a",
            )?;
            let recovery_request = read_request(&mut socket)?;
            send_error(
                &mut socket,
                RECOVERY_THREAD_RESUME_REQUEST_ID,
                -32000,
                "resume rejected",
            )?;
            Ok(vec![snapshot_request, turn_request, recovery_request])
        });

        let result = result_classification(send_and_confirm(
            &mut client_socket,
            THREAD_ID,
            "hello there",
            None,
            CLIENT_USER_MESSAGE_ID,
        ));
        let requests = join_server(&client_socket, &Err(result.1.clone()), server)?;
        let expected_requests = vec![
            thread_read_request(PRE_TURN_READ_REQUEST_ID),
            super::super::turn_start_request(
                THREAD_ID,
                "hello there",
                None,
                CLIENT_USER_MESSAGE_ID,
            )
            .payload,
            thread_resume_request_with_id(THREAD_ID, RECOVERY_THREAD_RESUME_REQUEST_ID).payload,
        ];

        assert_eq!(
            (result, requests),
            (
                (
                    "not_delivered",
                    "Codex app-server did not deliver the message: could not resume Codex thread after it was not loaded: Codex app-server `thread/resume` failed: resume rejected (code -32000)".to_string(),
                ),
                expected_requests,
            ),
        );
        Ok(())
    }

    #[rstest]
    fn does_not_retry_when_retried_turn_start_is_rejected(
        connected_sockets: anyhow::Result<(WebSocket<UnixStream>, WebSocket<UnixStream>)>,
    ) -> anyhow::Result<()> {
        let (mut client_socket, server_socket) = connected_sockets?;
        let server = std::thread::spawn(move || -> anyhow::Result<Vec<Value>> {
            let mut socket = server_socket;
            let snapshot_request = read_empty_snapshot(&mut socket)?;
            let first_turn_request = read_request(&mut socket)?;
            send_error(
                &mut socket,
                TURN_START_REQUEST_ID,
                -32600,
                "thread not found: thread-a",
            )?;
            let recovery_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                RECOVERY_THREAD_RESUME_REQUEST_ID,
                json!({"thread": {"turns": []}}),
            )?;
            let retry_request = read_request(&mut socket)?;
            send_error(
                &mut socket,
                RETRY_TURN_START_REQUEST_ID,
                -32600,
                "retry rejected",
            )?;
            Ok(vec![
                snapshot_request,
                first_turn_request,
                recovery_request,
                retry_request,
            ])
        });

        let result = result_classification(send_and_confirm(
            &mut client_socket,
            THREAD_ID,
            "hello there",
            None,
            CLIENT_USER_MESSAGE_ID,
        ));
        let requests = join_server(&client_socket, &Err(result.1.clone()), server)?;
        let expected_requests = vec![
            thread_read_request(PRE_TURN_READ_REQUEST_ID),
            super::super::turn_start_request(
                THREAD_ID,
                "hello there",
                None,
                CLIENT_USER_MESSAGE_ID,
            )
            .payload,
            thread_resume_request_with_id(THREAD_ID, RECOVERY_THREAD_RESUME_REQUEST_ID).payload,
            super::super::turn_start_request_with_id(
                RETRY_TURN_START_REQUEST_ID,
                THREAD_ID,
                "hello there",
                None,
                CLIENT_USER_MESSAGE_ID,
            )
            .payload,
        ];

        assert_eq!(
            (result, requests),
            (
                (
                    "not_delivered",
                    "Codex app-server did not deliver the message: Codex app-server `turn/start` failed: retry rejected (code -32600)".to_string(),
                ),
                expected_requests,
            ),
        );
        Ok(())
    }

    #[rstest]
    #[case::initial_start(false)]
    #[case::retry_start(true)]
    fn turn_start_disconnect_is_unconfirmed_without_an_extra_retry(
        connected_sockets: anyhow::Result<(WebSocket<UnixStream>, WebSocket<UnixStream>)>,
        #[case] disconnect_on_retry: bool,
    ) -> anyhow::Result<()> {
        let (mut client_socket, server_socket) = connected_sockets?;
        let server = std::thread::spawn(move || -> anyhow::Result<Vec<Value>> {
            let mut socket = server_socket;
            let snapshot_request = read_empty_snapshot(&mut socket)?;
            let first_turn_request = read_request(&mut socket)?;
            let mut requests = vec![snapshot_request, first_turn_request];
            if disconnect_on_retry {
                send_error(
                    &mut socket,
                    TURN_START_REQUEST_ID,
                    -32600,
                    "thread not found: thread-a",
                )?;
                requests.push(read_request(&mut socket)?);
                send_response(
                    &mut socket,
                    RECOVERY_THREAD_RESUME_REQUEST_ID,
                    json!({"thread": {"turns": []}}),
                )?;
                requests.push(read_request(&mut socket)?);
            }
            socket.send(Message::Close(None))?;
            Ok(requests)
        });

        let result = result_classification(send_and_confirm(
            &mut client_socket,
            THREAD_ID,
            "hello there",
            None,
            CLIENT_USER_MESSAGE_ID,
        ));
        let requests = join_server(&client_socket, &Err(result.1.clone()), server)?;
        let mut expected_requests = vec![
            thread_read_request(PRE_TURN_READ_REQUEST_ID),
            super::super::turn_start_request(
                THREAD_ID,
                "hello there",
                None,
                CLIENT_USER_MESSAGE_ID,
            )
            .payload,
        ];
        if disconnect_on_retry {
            expected_requests.extend([
                thread_resume_request_with_id(THREAD_ID, RECOVERY_THREAD_RESUME_REQUEST_ID).payload,
                super::super::turn_start_request_with_id(
                    RETRY_TURN_START_REQUEST_ID,
                    THREAD_ID,
                    "hello there",
                    None,
                    CLIENT_USER_MESSAGE_ID,
                )
                .payload,
            ]);
        }

        assert_eq!(
            (result, requests),
            (
                (
                    "unconfirmed",
                    "Codex app-server delivery is unconfirmed: Codex app-server closed during `turn/start`: None".to_string(),
                ),
                expected_requests,
            ),
        );
        Ok(())
    }

    #[test]
    fn reports_not_delivered_when_pre_turn_read_response_has_no_thread() -> anyhow::Result<()> {
        let (mut client_socket, server_socket) = connected_sockets()?;
        let server = std::thread::spawn(move || -> anyhow::Result<Vec<Value>> {
            let mut socket = server_socket;
            let snapshot_request = read_snapshot(&mut socket, json!({"unavailable": true}))?;
            Ok(vec![snapshot_request])
        });

        let result = result_text(send_and_confirm(
            &mut client_socket,
            THREAD_ID,
            "hello there",
            None,
            CLIENT_USER_MESSAGE_ID,
        ));
        let requests = join_server(&client_socket, &result, server)?;

        assert_eq!(
            (result, requests),
            (
                Err("Codex app-server did not deliver the message: Codex app-server thread/read response did not contain a thread before turn/start".to_string()),
                vec![thread_read_request(PRE_TURN_READ_REQUEST_ID)],
            ),
        );
        Ok(())
    }

    #[rstest]
    #[case::interrupted("interrupted")]
    #[case::completed("completed")]
    fn reports_not_delivered_after_terminal_history_omits_message(
        connected_sockets: anyhow::Result<(WebSocket<UnixStream>, WebSocket<UnixStream>)>,
        #[case] status: &str,
    ) -> anyhow::Result<()> {
        let (mut client_socket, server_socket) = connected_sockets?;
        let status = status.to_string();
        let server = std::thread::spawn(move || -> anyhow::Result<Vec<Value>> {
            let mut socket = server_socket;
            let snapshot_request = read_empty_snapshot(&mut socket)?;
            let turn_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                TURN_START_REQUEST_ID,
                json!({"turn": {"id": TURN_ID, "status": "inProgress"}}),
            )?;
            let resume_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                THREAD_RESUME_REQUEST_ID,
                thread_response("inProgress", None, None),
            )?;
            let history_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                THREAD_READ_REQUEST_ID,
                thread_response(&status, None, None),
            )?;
            let final_history_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                NEXT_THREAD_READ_REQUEST_ID,
                thread_response(&status, None, None),
            )?;
            Ok(vec![
                snapshot_request,
                turn_request,
                resume_request,
                history_request,
                final_history_request,
            ])
        });

        let result = result_text(send_and_confirm(
            &mut client_socket,
            THREAD_ID,
            "hello there",
            None,
            CLIENT_USER_MESSAGE_ID,
        ));
        let requests = join_server(&client_socket, &result, server)?;
        let expected_requests = start_and_subscribe_requests()
            .into_iter()
            .chain([
                thread_read_request(THREAD_READ_REQUEST_ID),
                thread_read_request(NEXT_THREAD_READ_REQUEST_ID),
            ])
            .collect::<Vec<_>>();

        assert_eq!(
            (result, requests),
            (
                Err("Codex app-server did not deliver the message: Codex app-server turn ended before a matching user message appeared in thread history".to_string()),
                expected_requests,
            ),
        );
        Ok(())
    }

    #[rstest]
    #[case::client_id(Some(CLIENT_USER_MESSAGE_ID), None)]
    #[case::matching_body(None, Some("hello there"))]
    fn does_not_resend_when_resume_history_contains_the_message(
        connected_sockets: anyhow::Result<(WebSocket<UnixStream>, WebSocket<UnixStream>)>,
        #[case] client_user_message_id: Option<&str>,
        #[case] content: Option<&str>,
    ) -> anyhow::Result<()> {
        let (mut client_socket, server_socket) = connected_sockets?;
        let client_user_message_id = client_user_message_id.map(str::to_owned);
        let content = content.map(str::to_owned);
        let server = std::thread::spawn(move || -> anyhow::Result<Vec<Value>> {
            let mut socket = server_socket;
            let snapshot_request = read_empty_snapshot(&mut socket)?;
            let turn_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                TURN_START_REQUEST_ID,
                json!({"turn": {"id": TURN_ID, "status": "inProgress"}}),
            )?;
            let resume_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                THREAD_RESUME_REQUEST_ID,
                thread_response(
                    "inProgress",
                    client_user_message_id.as_deref(),
                    content.as_deref(),
                ),
            )?;
            Ok(vec![snapshot_request, turn_request, resume_request])
        });

        let result = result_text(send_and_confirm(
            &mut client_socket,
            THREAD_ID,
            "hello there",
            None,
            CLIENT_USER_MESSAGE_ID,
        ));
        let requests = join_server(&client_socket, &result, server)?;

        assert_eq!((result, requests), (Ok(()), start_and_subscribe_requests()));
        Ok(())
    }

    #[test]
    fn confirms_history_after_turn_completed_notification() -> anyhow::Result<()> {
        let (mut client_socket, server_socket) = connected_sockets()?;
        let server = std::thread::spawn(move || -> anyhow::Result<Vec<Value>> {
            let mut socket = server_socket;
            let snapshot_request = read_empty_snapshot(&mut socket)?;
            let turn_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                TURN_START_REQUEST_ID,
                json!({"turn": {"id": TURN_ID, "status": "inProgress"}}),
            )?;
            let resume_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                THREAD_RESUME_REQUEST_ID,
                thread_response("inProgress", None, None),
            )?;
            let history_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                THREAD_READ_REQUEST_ID,
                thread_response("inProgress", None, None),
            )?;
            socket.send(Message::Text(
                json!({
                    "method": "turn/completed",
                    "params": {
                        "threadId": THREAD_ID,
                        "turn": {
                            "id": TURN_ID,
                            "status": "interrupted",
                            "items": [],
                        },
                    },
                })
                .to_string()
                .into(),
            ))?;
            let final_history_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                NEXT_THREAD_READ_REQUEST_ID,
                thread_response("inProgress", None, None),
            )?;
            Ok(vec![
                snapshot_request,
                turn_request,
                resume_request,
                history_request,
                final_history_request,
            ])
        });

        let result = result_text(send_and_confirm(
            &mut client_socket,
            THREAD_ID,
            "hello there",
            None,
            CLIENT_USER_MESSAGE_ID,
        ));
        let requests = join_server(&client_socket, &result, server)?;
        let expected_requests = start_and_subscribe_requests()
            .into_iter()
            .chain([
                thread_read_request(THREAD_READ_REQUEST_ID),
                thread_read_request(NEXT_THREAD_READ_REQUEST_ID),
            ])
            .collect::<Vec<_>>();

        assert_eq!(
            (result, requests),
            (
                Err("Codex app-server did not deliver the message: Codex app-server turn ended before a matching user message appeared in thread history".to_string()),
                expected_requests,
            ),
        );
        Ok(())
    }

    #[test]
    fn does_not_queue_when_final_history_contains_the_message() -> anyhow::Result<()> {
        let (mut client_socket, server_socket) = connected_sockets()?;
        let server = std::thread::spawn(move || -> anyhow::Result<Vec<Value>> {
            let mut socket = server_socket;
            let snapshot_request = read_empty_snapshot(&mut socket)?;
            let turn_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                TURN_START_REQUEST_ID,
                json!({"turn": {"id": TURN_ID, "status": "inProgress"}}),
            )?;
            let resume_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                THREAD_RESUME_REQUEST_ID,
                thread_response("inProgress", None, None),
            )?;
            let history_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                THREAD_READ_REQUEST_ID,
                thread_response("inProgress", None, None),
            )?;
            socket.send(Message::Text(
                json!({
                    "method": "turn/completed",
                    "params": {
                        "threadId": THREAD_ID,
                        "turn": {
                            "id": TURN_ID,
                            "status": "interrupted",
                            "items": [],
                        },
                    },
                })
                .to_string()
                .into(),
            ))?;
            let final_history_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                NEXT_THREAD_READ_REQUEST_ID,
                thread_response("interrupted", None, Some("hello there")),
            )?;
            Ok(vec![
                snapshot_request,
                turn_request,
                resume_request,
                history_request,
                final_history_request,
            ])
        });

        let result = result_text(send_and_confirm(
            &mut client_socket,
            THREAD_ID,
            "hello there",
            None,
            CLIENT_USER_MESSAGE_ID,
        ));
        let requests = join_server(&client_socket, &result, server)?;
        let expected_requests = start_and_subscribe_requests()
            .into_iter()
            .chain([
                thread_read_request(THREAD_READ_REQUEST_ID),
                thread_read_request(NEXT_THREAD_READ_REQUEST_ID),
            ])
            .collect::<Vec<_>>();

        assert_eq!((result, requests), (Ok(()), expected_requests));
        Ok(())
    }

    #[rstest]
    #[case::snapshot_available(
        false,
        "Codex app-server did not deliver the message: Codex app-server turn ended before a matching user message appeared in thread history"
    )]
    #[case::snapshot_rejected(
        true,
        "Codex app-server delivery is unconfirmed: Codex app-server turn ended without a matching client user message ID in thread history"
    )]
    fn ignores_same_body_message_without_a_new_client_id(
        #[case] reject_snapshot: bool,
        #[case] expected_result: &str,
    ) -> anyhow::Result<()> {
        let (mut client_socket, server_socket) = connected_sockets()?;
        let prior_message = json!({
            "thread": {"turns": [{
                "id": TURN_ID,
                "status": "inProgress",
                "items": [{
                    "type": "userMessage",
                    "id": "item-old",
                    "content": [{"type": "text", "text": "hello there"}],
                }],
            }]},
        });
        let server = std::thread::spawn(move || -> anyhow::Result<Vec<Value>> {
            let mut socket = server_socket;
            let snapshot_request = read_request(&mut socket)?;
            if reject_snapshot {
                send_error(
                    &mut socket,
                    PRE_TURN_READ_REQUEST_ID,
                    -32000,
                    "thread history is temporarily unavailable",
                )?;
            } else {
                send_response(&mut socket, PRE_TURN_READ_REQUEST_ID, prior_message)?;
            }
            let turn_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                TURN_START_REQUEST_ID,
                json!({"turn": {"id": TURN_ID, "status": "inProgress"}}),
            )?;
            let resume_request = read_request(&mut socket)?;
            socket.send(Message::Text(
                json!({
                    "method": "turn/completed",
                    "params": {
                        "threadId": THREAD_ID,
                        "turn": {"id": TURN_ID, "status": "interrupted", "items": []},
                    },
                })
                .to_string()
                .into(),
            ))?;
            send_response(
                &mut socket,
                THREAD_RESUME_REQUEST_ID,
                thread_response("interrupted", None, Some("hello there")),
            )?;
            let history_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                THREAD_READ_REQUEST_ID,
                thread_response("interrupted", None, Some("hello there")),
            )?;
            let final_history_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                NEXT_THREAD_READ_REQUEST_ID,
                thread_response("interrupted", None, Some("hello there")),
            )?;
            Ok(vec![
                snapshot_request,
                turn_request,
                resume_request,
                history_request,
                final_history_request,
            ])
        });

        let result = result_text(send_and_confirm(
            &mut client_socket,
            THREAD_ID,
            "hello there",
            None,
            CLIENT_USER_MESSAGE_ID,
        ));
        let requests = join_server(&client_socket, &result, server)?;
        let expected_requests = start_and_subscribe_requests()
            .into_iter()
            .chain([
                thread_read_request(THREAD_READ_REQUEST_ID),
                thread_read_request(NEXT_THREAD_READ_REQUEST_ID),
            ])
            .collect::<Vec<_>>();

        assert_eq!(
            (result, requests),
            (Err(expected_result.to_string()), expected_requests,),
        );
        Ok(())
    }

    #[test]
    fn polls_thread_history_after_a_read_timeout() -> anyhow::Result<()> {
        let (mut client_socket, server_socket) = connected_sockets()?;
        let server = std::thread::spawn(move || -> anyhow::Result<Vec<Value>> {
            let mut socket = server_socket;
            let snapshot_request = read_empty_snapshot(&mut socket)?;
            let turn_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                TURN_START_REQUEST_ID,
                json!({"turn": {"id": TURN_ID, "status": "inProgress"}}),
            )?;
            let resume_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                THREAD_RESUME_REQUEST_ID,
                thread_response("inProgress", None, None),
            )?;
            let history_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                THREAD_READ_REQUEST_ID,
                thread_response("inProgress", None, None),
            )?;
            let polled_history_request = read_request(&mut socket)?;
            send_response(
                &mut socket,
                NEXT_THREAD_READ_REQUEST_ID,
                thread_response("inProgress", None, Some("hello there")),
            )?;
            Ok(vec![
                snapshot_request,
                turn_request,
                resume_request,
                history_request,
                polled_history_request,
            ])
        });

        let mut should_timeout = true;
        let mut poll_timeout = None;
        let result = result_text(send_and_confirm_with_reader(
            &mut client_socket,
            THREAD_ID,
            "hello there",
            None,
            CLIENT_USER_MESSAGE_ID,
            |socket| {
                poll_timeout = socket.get_ref().read_timeout().ok().flatten();
                if std::mem::replace(&mut should_timeout, false) {
                    Err(tungstenite::Error::Io(std::io::Error::from(
                        ErrorKind::WouldBlock,
                    )))
                } else {
                    Err(tungstenite::Error::ConnectionClosed)
                }
            },
        ));
        let requests = join_server(&client_socket, &result, server)?;
        let expected_requests = start_and_subscribe_requests()
            .into_iter()
            .chain([
                thread_read_request(THREAD_READ_REQUEST_ID),
                thread_read_request(NEXT_THREAD_READ_REQUEST_ID),
            ])
            .collect::<Vec<_>>();

        assert_eq!(
            (result, requests, poll_timeout),
            (Ok(()), expected_requests, Some(Duration::from_secs(5)),),
        );
        Ok(())
    }
}
