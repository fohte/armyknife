use anyhow::anyhow;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Client, RpcRequest, request_error_to_anyhow};

impl Client {
    pub(crate) fn read_subagent_thread(
        &mut self,
        thread_id: &str,
        include_turns: bool,
    ) -> anyhow::Result<Value> {
        let request_id = self.next_request_id()?;
        let request = thread_read_request(thread_id, request_id, include_turns);
        let (response, _) =
            super::send_request_and_collect_notifications(&mut self.socket, &request)
                .map_err(request_error_to_anyhow)?;
        Ok(response)
    }

    pub(crate) fn queue_subagent_message(
        &mut self,
        thread_id: &str,
        content: &str,
    ) -> anyhow::Result<()> {
        let request_id = self.next_request_id()?;
        let request = queue_add_request(thread_id, content, request_id);
        let response = super::send_request_response(&mut self.socket, &request)
            .map_err(request_error_to_anyhow)?;
        if response.get("queuedSubmission").is_none() {
            return Err(anyhow!(
                "Codex app-server thread/queue/add response did not contain a queued submission"
            ));
        }
        Ok(())
    }
}

fn thread_read_request(thread_id: &str, request_id: u64, include_turns: bool) -> RpcRequest {
    RpcRequest {
        id: request_id,
        method: "thread/read",
        payload: json!({
            "id": request_id,
            "method": "thread/read",
            "params": {
                "threadId": thread_id,
                "includeTurns": include_turns,
            },
        }),
    }
}

fn queue_add_request(thread_id: &str, content: &str, request_id: u64) -> RpcRequest {
    RpcRequest {
        id: request_id,
        method: "thread/queue/add",
        payload: json!({
            "id": request_id,
            "method": "thread/queue/add",
            "params": {
                "threadId": thread_id,
                "clientUserMessageId": Uuid::new_v4().to_string(),
                "input": [{
                    "type": "text",
                    "text": content,
                    "textElements": [],
                }],
            },
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;

    use anyhow::anyhow;
    use rstest::{fixture, rstest};
    use serde_json::{Value, json};
    use tungstenite::{Message, WebSocket, client};

    use super::super::{Client, TURN_STATUS_READ_REQUEST_ID_START, test_support};

    #[fixture]
    fn connected_client_and_server() -> (Client, WebSocket<UnixStream>) {
        let (client_stream, server_stream) = UnixStream::pair().expect("socket pair");
        let server =
            std::thread::spawn(move || test_support::accept_with_read_timeout(server_stream));
        let (socket, _) = client("ws://localhost/rpc", client_stream).expect("WebSocket client");
        let server = server
            .join()
            .expect("Codex app-server test thread")
            .expect("WebSocket server");

        (
            Client {
                socket,
                next_request_id: TURN_STATUS_READ_REQUEST_ID_START,
            },
            server,
        )
    }

    fn request_value(message: Message) -> anyhow::Result<Value> {
        let text = message
            .into_text()
            .map_err(|error| anyhow!("request was not text: {error}"))?;
        Ok(serde_json::from_str(&text)?)
    }

    #[rstest]
    #[case::with_turns(true)]
    #[case::without_turns(false)]
    fn reads_subagent_thread_with_requested_turn_history(
        #[case] include_turns: bool,
        connected_client_and_server: (Client, WebSocket<UnixStream>),
    ) {
        let (mut client, mut server) = connected_client_and_server;
        let server_thread = std::thread::spawn(move || {
            let request =
                request_value(server.read().expect("thread/read request")).expect("request JSON");
            server
                .send(Message::Text(
                    json!({
                        "id": TURN_STATUS_READ_REQUEST_ID_START,
                        "result": {"thread": {"id": "sample-thread"}},
                    })
                    .to_string()
                    .into(),
                ))
                .expect("thread/read response");
            request
        });

        let response = client
            .read_subagent_thread("sample-thread", include_turns)
            .expect("thread snapshot");
        let request = server_thread.join().expect("server thread");

        assert_eq!(
            (request, response),
            (
                json!({
                    "id": TURN_STATUS_READ_REQUEST_ID_START,
                    "method": "thread/read",
                    "params": {
                        "threadId": "sample-thread",
                        "includeTurns": include_turns,
                    },
                }),
                json!({"thread": {"id": "sample-thread"}}),
            ),
        );
    }

    #[rstest]
    fn adds_a_message_with_the_queue_only_request(
        connected_client_and_server: (Client, WebSocket<UnixStream>),
    ) {
        let (mut client, mut server) = connected_client_and_server;
        let server_thread = std::thread::spawn(move || {
            let mut request =
                request_value(server.read().expect("queue/add request")).expect("request JSON");
            request["params"]["clientUserMessageId"] = json!("sample-message-id");
            server
                .send(Message::Text(
                    json!({
                        "id": TURN_STATUS_READ_REQUEST_ID_START,
                        "result": {"queuedSubmission": {"id": "sample-queue-id"}},
                    })
                    .to_string()
                    .into(),
                ))
                .expect("queue/add response");
            request
        });

        let result = client
            .queue_subagent_message("sample-parent-thread", "A child Codex agent has completed.");
        let request = server_thread.join().expect("server thread");

        assert_eq!(
            (request, result.map_err(|error| error.to_string())),
            (
                json!({
                    "id": TURN_STATUS_READ_REQUEST_ID_START,
                    "method": "thread/queue/add",
                    "params": {
                        "threadId": "sample-parent-thread",
                        "clientUserMessageId": "sample-message-id",
                        "input": [{
                            "type": "text",
                            "text": "A child Codex agent has completed.",
                            "textElements": [],
                        }],
                    },
                }),
                Ok(()),
            ),
        );
    }

    #[rstest]
    #[case::missing_queue_submission(
        json!({"result": {}}),
        Err("Codex app-server thread/queue/add response did not contain a queued submission".to_string()),
    )]
    #[case::rejected_queue_submission(
        json!({"error": {"code": 409, "message": "queue rejected"}}),
        Err("Codex app-server `thread/queue/add` failed: queue rejected (code 409)".to_string()),
    )]
    fn reports_queue_add_failures(
        #[case] response: Value,
        #[case] expected_error: std::result::Result<(), String>,
        connected_client_and_server: (Client, WebSocket<UnixStream>),
    ) {
        let (mut client, mut server) = connected_client_and_server;
        let server_thread = std::thread::spawn(move || {
            let mut request =
                request_value(server.read().expect("queue/add request")).expect("request JSON");
            request["params"]["clientUserMessageId"] = json!("sample-message-id");
            let mut response = response;
            response["id"] = json!(TURN_STATUS_READ_REQUEST_ID_START);
            server
                .send(Message::Text(response.to_string().into()))
                .expect("queue/add response");
            request
        });

        let result = client
            .queue_subagent_message("sample-parent-thread", "A child Codex agent has completed.")
            .map_err(|error| error.to_string());
        let request = server_thread.join().expect("server thread");

        assert_eq!(
            (request, result),
            (
                json!({
                    "id": TURN_STATUS_READ_REQUEST_ID_START,
                    "method": "thread/queue/add",
                    "params": {
                        "threadId": "sample-parent-thread",
                        "clientUserMessageId": "sample-message-id",
                        "input": [{
                            "type": "text",
                            "text": "A child Codex agent has completed.",
                            "textElements": [],
                        }],
                    },
                }),
                expected_error,
            ),
        );
    }
}
