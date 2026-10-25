use std::collections::HashSet;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};

use super::{Client, RpcRequest, request_error_to_anyhow};
use response::{
    is_terminal_turn_status, next_cursor, page_data, validate_item_entry, validate_turn,
};

mod response;

const PAGE_SIZE: u64 = 100;
const MAX_PAGES: usize = 1_000;
const TURN_COMPLETION_TIMEOUT: Duration = Duration::from_secs(15);
const HISTORY_FETCH_TIMEOUT: Duration = Duration::from_secs(60);
const FINAL_MESSAGE_REFLECTION_TIMEOUT: Duration = Duration::from_secs(10);
const TURN_POLL_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, Copy)]
struct HistoryTimeouts {
    turn_completion: Duration,
    fetch: Duration,
    final_message_reflection: Duration,
}

const HISTORY_TIMEOUTS: HistoryTimeouts = HistoryTimeouts {
    turn_completion: TURN_COMPLETION_TIMEOUT,
    fetch: HISTORY_FETCH_TIMEOUT,
    final_message_reflection: FINAL_MESSAGE_REFLECTION_TIMEOUT,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CodexThreadMetadata {
    pub(super) model_provider: String,
    pub(super) model: String,
    pub(super) session_id: String,
}

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

    pub(super) fn read_complete_thread(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        last_assistant_message: &str,
    ) -> Result<CodexThreadMetadata> {
        let mut request =
            |method, params, timeout| self.request_with_timeout(method, params, timeout);
        read_complete_thread_with(
            thread_id,
            turn_id,
            last_assistant_message,
            &mut request,
            &mut thread::sleep,
            HISTORY_TIMEOUTS,
        )
    }
}

fn read_complete_thread_with<Rpc, Sleep>(
    thread_id: &str,
    turn_id: &str,
    last_assistant_message: &str,
    rpc: &mut Rpc,
    sleep: &mut Sleep,
    timeouts: HistoryTimeouts,
) -> Result<CodexThreadMetadata>
where
    Rpc: FnMut(&'static str, Value, Duration) -> Result<Value>,
    Sleep: FnMut(Duration),
{
    ensure!(!thread_id.is_empty(), "Codex thread ID is empty");
    ensure!(!turn_id.is_empty(), "Codex turn ID is empty");
    ensure!(
        !last_assistant_message.trim().is_empty(),
        "Codex final assistant message is empty"
    );

    let metadata_deadline = deadline_after(timeouts.fetch)?;
    let metadata = rpc(
        "thread/read",
        json!({ "threadId": thread_id, "includeTurns": false }),
        remaining(metadata_deadline)?,
    )?;
    let metadata_thread = metadata
        .get("thread")
        .context("Codex app-server thread/read response has no thread")?;
    ensure!(
        metadata_thread.get("id").and_then(Value::as_str) == Some(thread_id),
        "Codex app-server thread/read returned a different thread"
    );
    let thread = CodexThreadMetadata {
        model_provider: metadata_thread
            .get("modelProvider")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .context("Codex app-server thread/read response has no model provider")?
            .to_string(),
        model: metadata_thread
            .get("model")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .context("Codex app-server thread/read response has no model")?
            .to_string(),
        session_id: metadata_thread
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .context("Codex app-server thread/read response has no session ID")?
            .to_string(),
    };

    let completion_deadline = deadline_after(timeouts.turn_completion)?;
    loop {
        let Some(status) = read_target_turn_status(rpc, thread_id, turn_id, completion_deadline)?
        else {
            wait_before_deadline(sleep, completion_deadline)?;
            continue;
        };
        if is_terminal_turn_status(&status) {
            break;
        }
        if status == "inProgress" {
            wait_before_deadline(sleep, completion_deadline)?;
        } else {
            bail!("Codex target turn has unsupported status `{status}`");
        }
    }

    let reflection_deadline = deadline_after(timeouts.final_message_reflection)?;
    loop {
        if read_last_assistant_message_match(
            rpc,
            thread_id,
            turn_id,
            last_assistant_message,
            reflection_deadline,
        )?
        .unwrap_or(false)
        {
            break;
        }
        wait_before_deadline(sleep, reflection_deadline)?;
    }

    Ok(thread)
}

fn read_target_turn_status<Rpc>(
    rpc: &mut Rpc,
    thread_id: &str,
    turn_id: &str,
    deadline: Instant,
) -> Result<Option<String>>
where
    Rpc: FnMut(&'static str, Value, Duration) -> Result<Value>,
{
    paginate(
        rpc,
        "thread/turns/list",
        deadline,
        |cursor| turn_page_params(thread_id, cursor),
        |data| {
            for turn in data {
                let (id, status) = validate_turn(turn)?;
                if id == turn_id {
                    return Ok(Some(status.to_string()));
                }
            }
            Ok(None)
        },
    )
}

fn read_last_assistant_message_match<Rpc>(
    rpc: &mut Rpc,
    thread_id: &str,
    turn_id: &str,
    expected: &str,
    deadline: Instant,
) -> Result<Option<bool>>
where
    Rpc: FnMut(&'static str, Value, Duration) -> Result<Value>,
{
    paginate(
        rpc,
        "thread/items/list",
        deadline,
        |cursor| item_page_params(thread_id, turn_id, cursor),
        |data| {
            for entry in data {
                validate_item_entry(entry)?;
                ensure!(
                    entry.get("turnId").and_then(Value::as_str) == Some(turn_id),
                    "Codex thread/items/list returned an item for a different turn"
                );
                if entry.pointer("/item/type").and_then(Value::as_str) == Some("agentMessage") {
                    let message = entry
                        .pointer("/item/text")
                        .and_then(Value::as_str)
                        .context("Codex assistant message item has no text")?;
                    return Ok(Some(message == expected));
                }
            }
            Ok(None)
        },
    )
}

fn turn_page_params(thread_id: &str, cursor: Option<&str>) -> Value {
    let mut params = json!({
        "threadId": thread_id,
        "limit": PAGE_SIZE,
        "sortDirection": "desc",
        "itemsView": "notLoaded",
    });
    if let Some(cursor) = cursor {
        params["cursor"] = json!(cursor);
    }
    params
}

fn item_page_params(thread_id: &str, turn_id: &str, cursor: Option<&str>) -> Value {
    let mut params = json!({
        "threadId": thread_id,
        "turnId": turn_id,
        "limit": PAGE_SIZE,
        "sortDirection": "desc",
    });
    if let Some(cursor) = cursor {
        params["cursor"] = json!(cursor);
    }
    params
}

fn paginate<Rpc, Params, HandlePage, T>(
    rpc: &mut Rpc,
    method: &'static str,
    deadline: Instant,
    mut params: Params,
    mut handle_page: HandlePage,
) -> Result<Option<T>>
where
    Rpc: FnMut(&'static str, Value, Duration) -> Result<Value>,
    Params: FnMut(Option<&str>) -> Value,
    HandlePage: FnMut(&[Value]) -> Result<Option<T>>,
{
    let mut cursor = None;
    let mut seen_cursors = HashSet::new();
    for _ in 0..MAX_PAGES {
        let response = rpc(method, params(cursor.as_deref()), remaining(deadline)?)?;
        let data = page_data(&response, method)?;
        if let Some(result) = handle_page(data)? {
            return Ok(Some(result));
        }
        cursor = next_cursor(&response, method)?;
        let Some(next) = cursor.as_deref() else {
            return Ok(None);
        };
        ensure!(
            seen_cursors.insert(next.to_string()),
            "Codex {method} repeated a pagination cursor"
        );
    }
    bail!("Codex {method} exceeded the {MAX_PAGES}-page limit")
}

fn deadline_after(timeout: Duration) -> Result<Instant> {
    Instant::now()
        .checked_add(timeout)
        .context("Codex history timeout is out of range")
}

fn remaining(deadline: Instant) -> Result<Duration> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    ensure!(!remaining.is_zero(), "Codex history request timed out");
    Ok(remaining)
}

fn wait_before_deadline<Sleep>(sleep: &mut Sleep, deadline: Instant) -> Result<()>
where
    Sleep: FnMut(Duration),
{
    let remaining = remaining(deadline)?;
    sleep(TURN_POLL_INTERVAL.min(remaining));
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::time::Duration;

    use anyhow::{Result, anyhow};
    use rstest::{fixture, rstest};
    use serde_json::{Value, json};

    use super::{CodexThreadMetadata, HistoryTimeouts, read_complete_thread_with};

    const THREAD_ID: &str = "fixture-thread";
    const TURN_ID: &str = "fixture-turn";
    const FINAL_MESSAGE: &str = "waiting for review";

    #[fixture]
    fn metadata_response() -> Value {
        json!({
            "thread": {
                "id": THREAD_ID,
                "modelProvider": "openai",
                "model": "fixture-model",
                "sessionId": "fixture-session",
            },
        })
    }

    fn turn(status: &str) -> Value {
        json!({ "id": TURN_ID, "status": status })
    }

    fn page(data: Vec<Value>, next_cursor: Option<&str>) -> Value {
        json!({ "data": data, "nextCursor": next_cursor })
    }

    fn item_entry(item: Value) -> Value {
        json!({ "turnId": TURN_ID, "item": item })
    }

    fn result_text(
        result: anyhow::Result<CodexThreadMetadata>,
    ) -> Result<CodexThreadMetadata, String> {
        result.map_err(|error| format!("{error:#}"))
    }

    fn history_timeouts(turn_completion: Duration) -> HistoryTimeouts {
        HistoryTimeouts {
            turn_completion,
            fetch: Duration::from_secs(1),
            final_message_reflection: Duration::from_secs(1),
        }
    }

    #[rstest]
    fn reads_only_thread_metadata_and_the_target_turn(metadata_response: Value) {
        let responses = VecDeque::from([
            ("thread/read", metadata_response),
            ("thread/turns/list", page(vec![turn("completed")], None)),
            (
                "thread/items/list",
                page(
                    vec![
                        item_entry(json!({ "id": "fixture-tool-item", "type": "toolCall" })),
                        item_entry(json!({
                            "id": "fixture-final-item",
                            "type": "agentMessage",
                            "text": FINAL_MESSAGE,
                        })),
                    ],
                    None,
                ),
            ),
        ]);
        let mut responses = responses;
        let mut calls = Vec::new();
        let mut sleeps = Vec::new();
        let mut rpc = |method: &'static str, params: Value, _timeout| {
            calls.push(json!({ "method": method, "params": params }));
            let (expected_method, response) = responses
                .pop_front()
                .ok_or_else(|| anyhow!("unexpected {method} request"))?;
            if method != expected_method {
                return Err(anyhow!("expected {expected_method}, got {method}"));
            }
            Ok(response)
        };
        let mut sleep = |duration| sleeps.push(duration);

        let actual = result_text(read_complete_thread_with(
            THREAD_ID,
            TURN_ID,
            FINAL_MESSAGE,
            &mut rpc,
            &mut sleep,
            history_timeouts(Duration::from_secs(1)),
        ));
        let expected_calls = vec![
            json!({
                "method": "thread/read",
                "params": { "threadId": THREAD_ID, "includeTurns": false },
            }),
            json!({
                "method": "thread/turns/list",
                "params": {
                    "threadId": THREAD_ID,
                    "limit": 100,
                    "sortDirection": "desc",
                    "itemsView": "notLoaded",
                },
            }),
            json!({
                "method": "thread/items/list",
                "params": {
                    "threadId": THREAD_ID,
                    "turnId": TURN_ID,
                    "limit": 100,
                    "sortDirection": "desc",
                },
            }),
        ];
        assert_eq!(
            (actual, calls, sleeps, responses,),
            (
                Ok(CodexThreadMetadata {
                    model_provider: "openai".to_string(),
                    model: "fixture-model".to_string(),
                    session_id: "fixture-session".to_string(),
                }),
                expected_calls,
                Vec::<Duration>::new(),
                VecDeque::new(),
            ),
        );
    }

    #[rstest]
    fn waits_for_turn_completion_and_the_final_message_to_be_saved(metadata_response: Value) {
        let responses = VecDeque::from([
            ("thread/read", metadata_response),
            ("thread/turns/list", page(vec![turn("inProgress")], None)),
            ("thread/turns/list", page(vec![turn("completed")], None)),
            (
                "thread/items/list",
                page(
                    vec![item_entry(json!({
                        "id": "fixture-latest-item",
                        "type": "agentMessage",
                        "text": "still working",
                    }))],
                    None,
                ),
            ),
            (
                "thread/items/list",
                page(
                    vec![item_entry(json!({
                        "id": "fixture-final-item",
                        "type": "agentMessage",
                        "text": FINAL_MESSAGE,
                    }))],
                    None,
                ),
            ),
        ]);
        let mut responses = responses;
        let mut calls = Vec::new();
        let mut sleeps = Vec::new();
        let mut rpc = |method: &'static str, params: Value, _timeout| {
            calls.push(json!({ "method": method, "params": params }));
            let (expected_method, response) = responses
                .pop_front()
                .ok_or_else(|| anyhow!("unexpected {method} request"))?;
            if method != expected_method {
                return Err(anyhow!("expected {expected_method}, got {method}"));
            }
            Ok(response)
        };
        let mut sleep = |duration| sleeps.push(duration);

        let actual = result_text(read_complete_thread_with(
            THREAD_ID,
            TURN_ID,
            FINAL_MESSAGE,
            &mut rpc,
            &mut sleep,
            history_timeouts(Duration::from_secs(1)),
        ));
        let expected_calls = vec![
            json!({ "method": "thread/read", "params": { "threadId": THREAD_ID, "includeTurns": false } }),
            json!({ "method": "thread/turns/list", "params": { "threadId": THREAD_ID, "limit": 100, "sortDirection": "desc", "itemsView": "notLoaded" } }),
            json!({ "method": "thread/turns/list", "params": { "threadId": THREAD_ID, "limit": 100, "sortDirection": "desc", "itemsView": "notLoaded" } }),
            json!({ "method": "thread/items/list", "params": { "threadId": THREAD_ID, "turnId": TURN_ID, "limit": 100, "sortDirection": "desc" } }),
            json!({ "method": "thread/items/list", "params": { "threadId": THREAD_ID, "turnId": TURN_ID, "limit": 100, "sortDirection": "desc" } }),
        ];
        assert_eq!(
            (actual, calls, sleeps, responses,),
            (
                Ok(CodexThreadMetadata {
                    model_provider: "openai".to_string(),
                    model: "fixture-model".to_string(),
                    session_id: "fixture-session".to_string(),
                }),
                expected_calls,
                vec![Duration::from_millis(250), Duration::from_millis(250)],
                VecDeque::new(),
            ),
        );
    }
}
