use std::collections::{BTreeMap, HashSet};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use serde_json::{Value, json};

use super::{Client, RpcRequest, request_error_to_anyhow};
use response::{
    add_history_bytes, is_terminal_turn_status, last_assistant_message_is_reflected, next_cursor,
    page_data, sanitize_nested_child_states, validate_item_entry, validate_item_turns,
    validate_turn,
};

mod response;

const PAGE_SIZE: u64 = 100;
const MAX_PAGES: usize = 1_000;
const MAX_HISTORY_BYTES: usize = 2 * 1024 * 1024;
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct ChildAgentState {
    thread_id: String,
    status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(super) struct CodexConversation {
    pub(super) target_turn_id: String,
    pub(super) last_assistant_message: String,
    pub(super) turns: Vec<Value>,
    pub(super) items: Vec<Value>,
    pub(super) child_agent_states: Vec<ChildAgentState>,
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
        let read_timeout = timeout.min(super::RESPONSE_TIMEOUT);
        self.socket
            .get_ref()
            .set_read_timeout(Some(read_timeout))
            .context("failed to set Codex app-server read timeout")?;
        self.socket
            .get_ref()
            .set_write_timeout(Some(read_timeout))
            .context("failed to set Codex app-server write timeout")?;
        let response = super::send_request_response(&mut self.socket, &request)
            .map_err(request_error_to_anyhow);
        self.socket
            .get_ref()
            .set_read_timeout(Some(super::RESPONSE_TIMEOUT))
            .context("failed to restore Codex app-server read timeout")?;
        self.socket
            .get_ref()
            .set_write_timeout(Some(super::RESPONSE_TIMEOUT))
            .context("failed to restore Codex app-server write timeout")?;
        response
    }

    pub(super) fn read_complete_conversation(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        last_assistant_message: &str,
    ) -> Result<CodexConversation> {
        let mut request =
            |method, params, timeout| self.request_with_timeout(method, params, timeout);
        read_complete_conversation_with(
            thread_id,
            turn_id,
            last_assistant_message,
            &mut request,
            &mut thread::sleep,
            HISTORY_TIMEOUTS,
        )
    }
}

fn read_complete_conversation_with<Rpc, Sleep>(
    thread_id: &str,
    turn_id: &str,
    last_assistant_message: &str,
    rpc: &mut Rpc,
    sleep: &mut Sleep,
    timeouts: HistoryTimeouts,
) -> Result<CodexConversation>
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

    let history_deadline = deadline_after(timeouts.fetch)?;
    let mut turns = fetch_all_turns(rpc, thread_id, history_deadline)?;
    let target_status = turns
        .iter()
        .find(|turn| turn.get("id").and_then(Value::as_str) == Some(turn_id))
        .and_then(|turn| turn.get("status"))
        .and_then(Value::as_str)
        .context("Codex turn pages did not contain the target turn")?;
    ensure!(
        is_terminal_turn_status(target_status),
        "Codex target turn is not complete"
    );

    let mut items = fetch_all_items(rpc, thread_id, history_deadline)?;
    if !last_assistant_message_is_reflected(&items, turn_id, last_assistant_message)? {
        let reflection_deadline = deadline_after(timeouts.final_message_reflection)?;
        loop {
            wait_before_deadline(sleep, reflection_deadline)?;
            items = fetch_all_items(rpc, thread_id, reflection_deadline)?;
            if last_assistant_message_is_reflected(&items, turn_id, last_assistant_message)? {
                break;
            }
        }
    }

    validate_item_turns(&items, &turns)?;
    let mut child_state_map = BTreeMap::new();
    for turn in &mut turns {
        sanitize_nested_child_states(turn, &mut child_state_map)?;
    }
    for item in &mut items {
        sanitize_nested_child_states(item, &mut child_state_map)?;
    }
    let conversation = CodexConversation {
        target_turn_id: turn_id.to_string(),
        last_assistant_message: last_assistant_message.to_string(),
        turns,
        items,
        child_agent_states: child_state_map
            .into_iter()
            .map(|(thread_id, status)| ChildAgentState { thread_id, status })
            .collect(),
    };
    let encoded = serde_json::to_vec(&conversation).context("serialize Codex conversation")?;
    ensure!(
        encoded.len() <= MAX_HISTORY_BYTES,
        "Codex conversation exceeds the {MAX_HISTORY_BYTES}-byte history limit"
    );
    Ok(conversation)
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
        |cursor| turn_page_params(thread_id, cursor, "desc", "notLoaded"),
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

fn fetch_all_turns<Rpc>(rpc: &mut Rpc, thread_id: &str, deadline: Instant) -> Result<Vec<Value>>
where
    Rpc: FnMut(&'static str, Value, Duration) -> Result<Value>,
{
    let mut turns = Vec::new();
    let mut turn_ids = HashSet::new();
    let mut history_bytes = 0usize;
    paginate(
        rpc,
        "thread/turns/list",
        deadline,
        |cursor| turn_page_params(thread_id, cursor, "asc", "summary"),
        |data| {
            for turn in data {
                let (id, _) = validate_turn(turn)?;
                ensure!(
                    turn_ids.insert(id.to_string()),
                    "Codex thread/turns/list returned a duplicate turn"
                );
                add_history_bytes(&mut history_bytes, turn)?;
                turns.push(turn.clone());
            }
            Ok(None::<()>)
        },
    )?;
    Ok(turns)
}

fn turn_page_params(
    thread_id: &str,
    cursor: Option<&str>,
    sort_direction: &str,
    items_view: &str,
) -> Value {
    let mut params = json!({
        "threadId": thread_id,
        "limit": PAGE_SIZE,
        "sortDirection": sort_direction,
        "itemsView": items_view,
    });
    if let Some(cursor) = cursor {
        params["cursor"] = json!(cursor);
    }
    params
}

fn fetch_all_items<Rpc>(rpc: &mut Rpc, thread_id: &str, deadline: Instant) -> Result<Vec<Value>>
where
    Rpc: FnMut(&'static str, Value, Duration) -> Result<Value>,
{
    let mut items = Vec::new();
    let mut item_ids = HashSet::new();
    let mut history_bytes = 0usize;
    paginate(
        rpc,
        "thread/items/list",
        deadline,
        |cursor| {
            let mut params = json!({
                "threadId": thread_id,
                "limit": PAGE_SIZE,
                "sortDirection": "asc",
            });
            if let Some(cursor) = cursor {
                params["cursor"] = json!(cursor);
            }
            params
        },
        |data| {
            for entry in data {
                validate_item_entry(entry)?;
                let item_id = entry
                    .pointer("/item/id")
                    .and_then(Value::as_str)
                    .context("Codex thread/items/list item has no ID")?;
                ensure!(
                    item_ids.insert(item_id.to_string()),
                    "Codex thread/items/list returned a duplicate item"
                );
                add_history_bytes(&mut history_bytes, entry)?;
                items.push(entry.clone());
            }
            Ok(None::<()>)
        },
    )?;
    Ok(items)
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

    use super::{
        ChildAgentState, CodexConversation, HistoryTimeouts, read_complete_conversation_with,
    };

    const THREAD_ID: &str = "fixture-thread";
    const TURN_ID: &str = "fixture-turn";
    const FINAL_MESSAGE: &str = "waiting for review";

    #[fixture]
    fn metadata_response() -> Value {
        json!({ "thread": { "id": THREAD_ID } })
    }

    fn turn(id: &str, status: &str) -> Value {
        json!({ "id": id, "status": status })
    }

    fn page(data: Vec<Value>, next_cursor: Option<&str>) -> Value {
        json!({ "data": data, "nextCursor": next_cursor })
    }

    fn item_entry(turn_id: &str, item: Value) -> Value {
        json!({ "turnId": turn_id, "item": item })
    }

    fn result_text(result: anyhow::Result<CodexConversation>) -> Result<CodexConversation, String> {
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
    fn loads_every_page_in_order_and_keeps_only_child_states(metadata_response: Value) {
        let user_item = item_entry(
            "fixture-old-turn",
            json!({
                "id": "fixture-user-item",
                "type": "userMessage",
                "content": [{ "type": "text", "text": "Continue the sample task." }],
            }),
        );
        let child_item = item_entry(
            TURN_ID,
            json!({
                "id": "fixture-child-item",
                "type": "collabAgentToolCall",
                "tool": "wait",
                "status": "inProgress",
                "senderThreadId": THREAD_ID,
                "receiverThreadIds": ["fixture-child-thread"],
                "agentsStates": {
                    "fixture-child-thread": {
                        "status": "running",
                        "message": "child output is not conversation input",
                    },
                },
            }),
        );
        let final_item = item_entry(
            TURN_ID,
            json!({
                "id": "fixture-final-item",
                "type": "agentMessage",
                "text": FINAL_MESSAGE,
            }),
        );
        let older_turn = json!({
            "id": "fixture-old-turn",
            "status": "completed",
            "summary": "earlier compressed summary",
        });
        let target_turn = turn(TURN_ID, "completed");
        let responses = VecDeque::from([
            ("thread/read", metadata_response),
            ("thread/turns/list", page(vec![target_turn.clone()], None)),
            (
                "thread/turns/list",
                page(vec![older_turn.clone()], Some("turn-cursor")),
            ),
            ("thread/turns/list", page(vec![target_turn.clone()], None)),
            (
                "thread/items/list",
                page(vec![user_item.clone(), child_item], Some("item-cursor")),
            ),
            ("thread/items/list", page(vec![final_item.clone()], None)),
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

        let actual = result_text(read_complete_conversation_with(
            THREAD_ID,
            TURN_ID,
            FINAL_MESSAGE,
            &mut rpc,
            &mut sleep,
            history_timeouts(Duration::from_secs(1)),
        ));
        let sanitized_child_item = item_entry(
            TURN_ID,
            json!({
                "id": "fixture-child-item",
                "type": "collabAgentToolCall",
                "tool": "wait",
                "status": "inProgress",
                "senderThreadId": THREAD_ID,
                "receiverThreadIds": ["fixture-child-thread"],
                "agentsStates": {
                    "fixture-child-thread": { "status": "running" },
                },
            }),
        );
        let expected = CodexConversation {
            target_turn_id: TURN_ID.to_string(),
            last_assistant_message: FINAL_MESSAGE.to_string(),
            turns: vec![older_turn, target_turn],
            items: vec![user_item, sanitized_child_item.clone(), final_item],
            child_agent_states: vec![ChildAgentState {
                thread_id: "fixture-child-thread".to_string(),
                status: "running".to_string(),
            }],
        };
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
                "method": "thread/turns/list",
                "params": {
                    "threadId": THREAD_ID,
                    "limit": 100,
                    "sortDirection": "asc",
                    "itemsView": "summary",
                },
            }),
            json!({
                "method": "thread/turns/list",
                "params": {
                    "threadId": THREAD_ID,
                    "limit": 100,
                    "sortDirection": "asc",
                    "itemsView": "summary",
                    "cursor": "turn-cursor",
                },
            }),
            json!({
                "method": "thread/items/list",
                "params": {
                    "threadId": THREAD_ID,
                    "limit": 100,
                    "sortDirection": "asc",
                },
            }),
            json!({
                "method": "thread/items/list",
                "params": {
                    "threadId": THREAD_ID,
                    "limit": 100,
                    "sortDirection": "asc",
                    "cursor": "item-cursor",
                },
            }),
        ];
        assert_eq!(
            (actual, calls, sleeps),
            (Ok(expected), expected_calls, Vec::<Duration>::new())
        );
    }

    #[rstest]
    fn waits_for_a_terminal_turn_and_the_final_message_to_be_saved(metadata_response: Value) {
        let pending = page(vec![turn(TURN_ID, "inProgress")], None);
        let completed = page(vec![turn(TURN_ID, "completed")], None);
        let turns = page(vec![turn(TURN_ID, "completed")], None);
        let stale_matching_message = page(
            vec![
                item_entry(
                    TURN_ID,
                    json!({ "id": "fixture-earlier-item", "type": "agentMessage", "text": FINAL_MESSAGE }),
                ),
                item_entry(
                    TURN_ID,
                    json!({ "id": "fixture-later-item", "type": "agentMessage", "text": "still working" }),
                ),
            ],
            None,
        );
        let reflected_message = page(
            vec![item_entry(
                TURN_ID,
                json!({ "id": "fixture-final-item", "type": "agentMessage", "text": FINAL_MESSAGE }),
            )],
            None,
        );
        let mut responses = VecDeque::from([
            ("thread/read", metadata_response),
            ("thread/turns/list", pending),
            ("thread/turns/list", completed),
            ("thread/turns/list", turns),
            ("thread/items/list", stale_matching_message),
            ("thread/items/list", reflected_message.clone()),
        ]);
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

        let result = read_complete_conversation_with(
            THREAD_ID,
            TURN_ID,
            FINAL_MESSAGE,
            &mut rpc,
            &mut sleep,
            history_timeouts(Duration::from_secs(1)),
        );
        let actual = result_text(result);
        let expected = CodexConversation {
            target_turn_id: TURN_ID.to_string(),
            last_assistant_message: FINAL_MESSAGE.to_string(),
            turns: vec![turn(TURN_ID, "completed")],
            items: vec![reflected_message["data"][0].clone()],
            child_agent_states: Vec::new(),
        };
        let expected_calls = vec![
            json!({ "method": "thread/read", "params": { "threadId": THREAD_ID, "includeTurns": false } }),
            json!({ "method": "thread/turns/list", "params": { "threadId": THREAD_ID, "limit": 100, "sortDirection": "desc", "itemsView": "notLoaded" } }),
            json!({ "method": "thread/turns/list", "params": { "threadId": THREAD_ID, "limit": 100, "sortDirection": "desc", "itemsView": "notLoaded" } }),
            json!({ "method": "thread/turns/list", "params": { "threadId": THREAD_ID, "limit": 100, "sortDirection": "asc", "itemsView": "summary" } }),
            json!({ "method": "thread/items/list", "params": { "threadId": THREAD_ID, "limit": 100, "sortDirection": "asc" } }),
            json!({ "method": "thread/items/list", "params": { "threadId": THREAD_ID, "limit": 100, "sortDirection": "asc" } }),
        ];
        assert_eq!(
            (actual, calls, sleeps),
            (
                Ok(expected),
                expected_calls,
                vec![Duration::from_millis(250), Duration::from_millis(250)],
            ),
        );
    }

    #[rstest]
    #[case::incomplete_turn(
        vec![page(vec![turn(TURN_ID, "inProgress")], None)],
        Duration::from_millis(1),
        "Codex history request timed out",
        1,
    )]
    #[case::missing_page_cursor(
        vec![
            page(vec![turn(TURN_ID, "completed")], None),
            json!({ "data": [] }),
        ],
        Duration::from_secs(1),
        "Codex app-server `thread/turns/list` response has no nextCursor",
        2,
    )]
    fn rejects_incomplete_or_unpageable_history(
        metadata_response: Value,
        #[case] turn_responses: Vec<Value>,
        #[case] turn_timeout: Duration,
        #[case] expected_error: &str,
        #[case] expected_turn_page_calls: usize,
    ) {
        let mut responses = VecDeque::from([("thread/read", metadata_response)]);
        responses.extend(
            turn_responses
                .into_iter()
                .map(|response| ("thread/turns/list", response)),
        );
        let mut calls = Vec::new();
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
        let mut sleep = std::thread::sleep;

        let actual = result_text(read_complete_conversation_with(
            THREAD_ID,
            TURN_ID,
            FINAL_MESSAGE,
            &mut rpc,
            &mut sleep,
            history_timeouts(turn_timeout),
        ));
        let expected = Err(expected_error.to_string());
        assert_eq!(
            (actual, calls.len()),
            (expected, expected_turn_page_calls + 1)
        );
    }
}
