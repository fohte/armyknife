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
    use serde_json::json;

    use super::turn_interrupt_request;

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
}
