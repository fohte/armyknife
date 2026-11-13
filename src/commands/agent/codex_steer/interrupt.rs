use anyhow::{Context, anyhow};
use serde_json::json;

use super::{Client, RpcRequest};

pub(crate) fn interrupt_turn(thread_id: &str) -> anyhow::Result<()> {
    let mut client = Client::connect()?;
    client.interrupt_latest_turn(thread_id)
}

impl Client {
    fn interrupt_latest_turn(&mut self, thread_id: &str) -> anyhow::Result<()> {
        let request_id = self.next_request_id()?;
        let Some((turn_id, status)) =
            super::delivery::latest_turn_details(&mut self.socket, thread_id, request_id)?
        else {
            return Err(anyhow!("Codex thread has no turns to interrupt"));
        };
        if status != "inProgress" {
            return Err(anyhow!("Codex thread has no running turn to interrupt"));
        }

        let request_id = self.next_request_id()?;
        super::send_request(
            &mut self.socket,
            &turn_interrupt_request(request_id, thread_id, &turn_id),
        )
        .map_err(super::request_error_to_anyhow)
        .context("failed to interrupt Codex turn")
    }
}

fn turn_interrupt_request(request_id: u64, thread_id: &str, turn_id: &str) -> RpcRequest {
    RpcRequest {
        id: request_id,
        method: "turn/interrupt",
        payload: json!({
            "id": request_id,
            "method": "turn/interrupt",
            "params": {
                "threadId": thread_id,
                "turnId": turn_id,
            },
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;

    use rstest::rstest;
    use serde_json::{Value, json};
    use tungstenite::{Message, WebSocket, client};

    use super::super::test_support::accept_with_read_timeout;
    use super::super::{Client, TURN_STATUS_READ_REQUEST_ID_START};
    use super::turn_interrupt_request;

    const THREAD_ID: &str = "fixture-thread";

    #[test]
    fn creates_an_interrupt_request_for_the_specific_turn() {
        let request = turn_interrupt_request(101, "fixture-thread", "fixture-turn");

        assert_eq!(
            request.payload,
            json!({
                "id": 101,
                "method": "turn/interrupt",
                "params": {
                    "threadId": "fixture-thread",
                    "turnId": "fixture-turn",
                },
            }),
        );
    }

    #[test]
    fn interrupts_the_latest_in_progress_turn() {
        let (mut client, mut server) = connected_client_and_server();
        let server_thread = std::thread::spawn(move || {
            let turns_request = read_request(&mut server);
            send_response(
                &mut server,
                TURN_STATUS_READ_REQUEST_ID_START,
                json!({
                    "data": [
                        {"id": "fixture-active-turn", "startedAt": 20, "status": "inProgress"},
                        {"id": "fixture-old-turn", "startedAt": 10, "status": "completed"},
                    ],
                    "nextCursor": null,
                    "backwardsCursor": null,
                }),
            );
            let interrupt_request = read_request(&mut server);
            send_response(
                &mut server,
                TURN_STATUS_READ_REQUEST_ID_START + 1,
                json!({}),
            );
            (turns_request, interrupt_request)
        });

        let result = client
            .interrupt_latest_turn(THREAD_ID)
            .map_err(|error| format!("{error:#}"));
        let requests = server_thread.join().expect("app-server thread");

        assert_eq!(
            (result, requests),
            (
                Ok(()),
                (
                    turns_list_request(),
                    json!({
                        "id": TURN_STATUS_READ_REQUEST_ID_START + 1,
                        "method": "turn/interrupt",
                        "params": {
                            "threadId": THREAD_ID,
                            "turnId": "fixture-active-turn",
                        },
                    }),
                ),
            ),
        );
    }

    #[rstest]
    #[case::empty_turn_list(json!([]), "Codex thread has no turns to interrupt")]
    #[case::terminal_turn(
        json!([{"id": "fixture-finished-turn", "startedAt": 20, "status": "completed"}]),
        "Codex thread has no running turn to interrupt"
    )]
    #[test]
    fn does_not_interrupt_without_an_in_progress_turn(
        #[case] turns: Value,
        #[case] expected_error: &str,
    ) {
        let (mut client, mut server) = connected_client_and_server();
        let server_thread = std::thread::spawn(move || {
            let request = read_request(&mut server);
            send_response(
                &mut server,
                TURN_STATUS_READ_REQUEST_ID_START,
                json!({
                    "data": turns,
                    "nextCursor": null,
                    "backwardsCursor": null,
                }),
            );
            request
        });

        let result = client
            .interrupt_latest_turn(THREAD_ID)
            .map_err(|error| format!("{error:#}"));
        let request = server_thread.join().expect("app-server thread");

        assert_eq!(
            (result, request),
            (Err(expected_error.to_string()), turns_list_request(),),
        );
    }

    fn connected_client_and_server() -> (Client, WebSocket<UnixStream>) {
        let (client_stream, server_stream) = UnixStream::pair().expect("socket pair");
        let server_thread = std::thread::spawn(move || accept_with_read_timeout(server_stream));
        let (socket, _) = client("ws://localhost/rpc", client_stream).expect("client socket");
        let server = server_thread
            .join()
            .expect("app-server handshake thread")
            .expect("app-server socket");

        (
            Client {
                socket,
                next_request_id: TURN_STATUS_READ_REQUEST_ID_START,
            },
            server,
        )
    }

    fn read_request(socket: &mut WebSocket<UnixStream>) -> Value {
        let Message::Text(request) = socket.read().expect("app-server request") else {
            panic!("Codex app-server received a non-text request");
        };
        serde_json::from_str(&request).expect("request JSON")
    }

    fn send_response(socket: &mut WebSocket<UnixStream>, request_id: u64, result: Value) {
        socket
            .send(Message::Text(
                json!({"id": request_id, "result": result})
                    .to_string()
                    .into(),
            ))
            .expect("app-server response");
    }

    fn turns_list_request() -> Value {
        json!({
            "id": TURN_STATUS_READ_REQUEST_ID_START,
            "method": "thread/turns/list",
            "params": {
                "threadId": THREAD_ID,
                "limit": 1,
                "sortDirection": "desc",
                "itemsView": "notLoaded",
            },
        })
    }
}
