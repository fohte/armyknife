use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

use super::{Client, RpcRequest, request_error_to_anyhow};

const HISTORY_FETCH_TIMEOUT: Duration = Duration::from_secs(60);

impl Client {
    fn request_with_timeout(
        &mut self,
        method: &'static str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value> {
        ensure!(!timeout.is_zero(), "Codex app-server request timed out");

        let request_id = self.next_request_id()?;
        let request = RpcRequest {
            id: request_id,
            method,
            payload: json!({
                "id": request_id,
                "method": method,
                "params": params,
            }),
        };
        let deadline = deadline_after(timeout)?;
        let write_timeout = deadline
            .saturating_duration_since(Instant::now())
            .min(super::RESPONSE_TIMEOUT);
        ensure!(
            !write_timeout.is_zero(),
            "Codex app-server request timed out"
        );
        self.socket
            .get_ref()
            .set_write_timeout(Some(write_timeout))
            .context("failed to set Codex app-server write timeout")?;
        let response = super::send_request_response_until(&mut self.socket, &request, deadline)
            .map_err(request_error_to_anyhow);
        self.socket
            .get_ref()
            .set_write_timeout(Some(super::RESPONSE_TIMEOUT))
            .context("failed to restore Codex app-server write timeout")?;
        response
    }
}

fn deadline_after(timeout: Duration) -> Result<Instant> {
    Instant::now()
        .checked_add(timeout)
        .context("Codex history timeout is out of range")
}

pub(super) fn fork_thread(thread_id: &str, before_turn_id: &str) -> Result<(String, PathBuf)> {
    ensure!(!thread_id.is_empty(), "Codex thread ID is empty");
    ensure!(!before_turn_id.is_empty(), "Codex turn ID is empty");
    let mut client = Client::connect()?;
    fork_thread_with(thread_id, before_turn_id, |method, params, timeout| {
        client.request_with_timeout(method, params, timeout)
    })
}

fn fork_thread_with<Rpc>(
    thread_id: &str,
    before_turn_id: &str,
    mut rpc: Rpc,
) -> Result<(String, PathBuf)>
where
    Rpc: FnMut(&'static str, Value, Duration) -> Result<Value>,
{
    ensure!(!thread_id.is_empty(), "Codex thread ID is empty");
    ensure!(!before_turn_id.is_empty(), "Codex turn ID is empty");
    let response = rpc(
        "thread/fork",
        json!({
            "threadId": thread_id,
            "beforeTurnId": before_turn_id,
        }),
        HISTORY_FETCH_TIMEOUT,
    )?;
    let thread = response
        .get("thread")
        .context("Codex app-server thread/fork response has no thread")?;
    let forked_id = thread
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .context("Codex app-server thread/fork response has no thread ID")?
        .to_owned();
    let transcript_path = thread
        .get("path")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .context("Codex app-server thread/fork response has no transcript path")?;
    ensure!(
        forked_id != thread_id,
        "Codex app-server returned the original thread"
    );
    Ok((forked_id, transcript_path))
}

#[cfg(test)]
mod fork_tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn forks_before_the_selected_turn() {
        let mut requests = Vec::new();
        let response = fork_thread_with("fixture-thread", "fixture-turn", |method, params, _| {
            requests.push(json!({"method": method, "params": params}));
            Ok(json!({
                "thread": {
                    "id": "fixture-fork",
                    "path": "/fixture/fork.jsonl"
                }
            }))
        });

        assert_eq!(
            (response.unwrap(), requests),
            (
                (
                    "fixture-fork".to_string(),
                    PathBuf::from("/fixture/fork.jsonl")
                ),
                vec![json!({
                    "method": "thread/fork",
                    "params": {
                        "threadId": "fixture-thread",
                        "beforeTurnId": "fixture-turn"
                    }
                })]
            ),
        );
    }
}
