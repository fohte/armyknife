use std::io::ErrorKind;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use anyhow::{Context, anyhow};
use serde_json::{Value, json};
use tungstenite::{Message, WebSocket};

use crate::commands::agent::types::ReasoningEffort;

use super::{DeliveryError, RequestError, RpcRequest, TURN_START_RESPONSE_TIMEOUT};
mod history;
use history::{
    MatchingItemSnapshot, has_completed_turn_notification, has_recorded_message,
    has_user_message_notification, is_terminal_turn_status, matching_user_message_snapshot,
    turn_status,
};

const PRE_TURN_READ_REQUEST_ID: u64 = 2;
pub(super) const THREAD_RESUME_REQUEST_ID: u64 = 4;
const THREAD_READ_REQUEST_ID: u64 = 5;
const NEXT_THREAD_READ_REQUEST_ID: u64 = 6;

struct DeliveryContext<'a> {
    thread_id: &'a str,
    turn_id: &'a str,
    client_user_message_id: &'a str,
    content: &'a str,
    snapshot: &'a MatchingItemSnapshot,
    read_timeout: Duration,
}

pub(super) fn send_and_confirm(
    socket: &mut WebSocket<UnixStream>,
    thread_id: &str,
    content: &str,
    effort: Option<ReasoningEffort>,
    client_user_message_id: &str,
) -> super::Result<()> {
    send_and_confirm_with_read_timeout(
        socket,
        thread_id,
        content,
        effort,
        client_user_message_id,
        super::RESPONSE_TIMEOUT,
    )
}

fn send_and_confirm_with_read_timeout(
    socket: &mut WebSocket<UnixStream>,
    thread_id: &str,
    content: &str,
    effort: Option<ReasoningEffort>,
    client_user_message_id: &str,
    read_timeout: Duration,
) -> super::Result<()> {
    let (snapshot_response, _) =
        read_thread(socket, thread_id, PRE_TURN_READ_REQUEST_ID, read_timeout).map_err(
            |error| {
                DeliveryError::NotDelivered(
                    super::request_error_to_anyhow(error)
                        .context("could not read Codex thread before starting the turn"),
                )
            },
        )?;
    let snapshot_thread = snapshot_response.get("thread").ok_or_else(|| {
        DeliveryError::NotDelivered(anyhow!(
            "Codex app-server thread/read response did not contain a thread before turn/start"
        ))
    })?;
    let snapshot = matching_user_message_snapshot(snapshot_thread, content);

    socket
        .get_ref()
        .set_read_timeout(Some(TURN_START_RESPONSE_TIMEOUT))
        .context("failed to set Codex app-server turn/start response timeout")
        .map_err(DeliveryError::NotDelivered)?;

    let turn_start_response = super::send_request_response(
        socket,
        &super::turn_start_request(thread_id, content, effort, client_user_message_id),
    )
    .map_err(start_request_error)?;
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
        read_timeout,
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
        read_thread(socket, thread_id, THREAD_READ_REQUEST_ID, read_timeout)
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
        let mut next_read_request_id = NEXT_THREAD_READ_REQUEST_ID;
        return confirm_after_turn_end(socket, &delivery, &mut next_read_request_id);
    }

    let mut next_read_request_id = NEXT_THREAD_READ_REQUEST_ID;
    match turn_status(thread, turn_id) {
        Some(status) if is_terminal_turn_status(status) => {
            confirm_after_turn_end(socket, &delivery, &mut next_read_request_id)
        }
        Some("inProgress") => wait_for_delivery(socket, &delivery, &mut next_read_request_id),
        Some(status) => Err(DeliveryError::Unconfirmed(anyhow!(
            "Codex app-server returned unknown turn status {status}"
        ))),
        None => wait_for_delivery(socket, &delivery, &mut next_read_request_id),
    }
}

fn wait_for_delivery(
    socket: &mut WebSocket<UnixStream>,
    delivery: &DeliveryContext<'_>,
    next_read_request_id: &mut u64,
) -> super::Result<()> {
    loop {
        let message = match socket.read() {
            Ok(message) => message,
            Err(error) if is_read_timeout(&error) => {
                let request_id = take_request_id(next_read_request_id)?;
                let (response, notifications) = read_thread(
                    socket,
                    delivery.thread_id,
                    request_id,
                    delivery.read_timeout,
                )
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
    let (response, notifications) = read_thread(
        socket,
        delivery.thread_id,
        request_id,
        delivery.read_timeout,
    )
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
    Err(not_delivered())
}

fn read_thread(
    socket: &mut WebSocket<UnixStream>,
    thread_id: &str,
    request_id: u64,
    read_timeout: Duration,
) -> std::result::Result<(Value, Vec<Value>), RequestError> {
    socket
        .get_ref()
        .set_read_timeout(Some(read_timeout))
        .context("failed to set Codex app-server thread/read response timeout")
        .map_err(RequestError::Transport)?;
    super::send_request_and_collect_notifications(
        socket,
        &thread_read_request(thread_id, request_id),
    )
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
    RpcRequest {
        id: THREAD_RESUME_REQUEST_ID,
        method: "thread/resume",
        payload: json!({
            "id": THREAD_RESUME_REQUEST_ID,
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
        RequestError::Rejected(error) => DeliveryError::NotDelivered(error),
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

    use super::super::TURN_START_REQUEST_ID;
    use super::*;

    const THREAD_ID: &str = "thread-a";
    const TURN_ID: &str = "turn-a";
    const CLIENT_USER_MESSAGE_ID: &str = "message-a";

    #[fixture]
    fn connected_sockets() -> anyhow::Result<(WebSocket<UnixStream>, WebSocket<UnixStream>)> {
        let (client_stream, server_stream) = UnixStream::pair()?;
        let server = std::thread::spawn(move || {
            tungstenite::accept(server_stream).map_err(anyhow::Error::from)
        });
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

    #[test]
    fn confirms_delivery_when_matching_user_message_starts() -> anyhow::Result<()> {
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
            Ok(vec![
                snapshot_request,
                turn_request,
                resume_request,
                history_request,
            ])
        });

        let result = result_text(send_and_confirm(
            &mut client_socket,
            THREAD_ID,
            "hello there",
            None,
            CLIENT_USER_MESSAGE_ID,
        ));
        let requests = server
            .join()
            .map_err(|_| anyhow!("Codex app-server test thread panicked"))??;
        let expected_requests = start_and_subscribe_requests()
            .into_iter()
            .chain(std::iter::once(thread_read_request(THREAD_READ_REQUEST_ID)))
            .collect::<Vec<_>>();

        assert_eq!((result, requests), (Ok(()), expected_requests));
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
        let requests = server
            .join()
            .map_err(|_| anyhow!("Codex app-server test thread panicked"))??;
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
                Err("Codex app-server turn ended before a matching user message appeared in thread history".to_string()),
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
        let requests = server
            .join()
            .map_err(|_| anyhow!("Codex app-server test thread panicked"))??;

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
        let requests = server
            .join()
            .map_err(|_| anyhow!("Codex app-server test thread panicked"))??;
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
                Err("Codex app-server turn ended before a matching user message appeared in thread history".to_string()),
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
        let requests = server
            .join()
            .map_err(|_| anyhow!("Codex app-server test thread panicked"))??;
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

    #[test]
    fn ignores_same_body_message_that_existed_before_the_turn() -> anyhow::Result<()> {
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
            let snapshot_request = read_snapshot(&mut socket, prior_message)?;
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
                thread_response("inProgress", None, Some("hello there")),
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
        let requests = server
            .join()
            .map_err(|_| anyhow!("Codex app-server test thread panicked"))??;
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
                Err("Codex app-server turn ended before a matching user message appeared in thread history".to_string()),
                expected_requests,
            ),
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

        let result = result_text(send_and_confirm_with_read_timeout(
            &mut client_socket,
            THREAD_ID,
            "hello there",
            None,
            CLIENT_USER_MESSAGE_ID,
            Duration::from_millis(10),
        ));
        let requests = server
            .join()
            .map_err(|_| anyhow!("Codex app-server test thread panicked"))??;
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
}
