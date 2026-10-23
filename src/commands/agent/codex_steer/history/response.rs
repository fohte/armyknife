use std::collections::{BTreeMap, HashSet};

use anyhow::{Context, Result, ensure};
use serde_json::Value;

use super::MAX_HISTORY_BYTES;

pub(super) fn page_data<'a>(response: &'a Value, method: &str) -> Result<&'a [Value]> {
    response
        .get("data")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .with_context(|| format!("Codex app-server `{method}` response has no data array"))
}

pub(super) fn next_cursor(response: &Value, method: &str) -> Result<Option<String>> {
    let value = response
        .get("nextCursor")
        .with_context(|| format!("Codex app-server `{method}` response has no nextCursor"))?;
    if value.is_null() {
        return Ok(None);
    }
    let cursor = value
        .as_str()
        .with_context(|| format!("Codex app-server `{method}` nextCursor is not a string"))?;
    ensure!(
        !cursor.is_empty(),
        "Codex app-server `{method}` returned an empty pagination cursor"
    );
    Ok(Some(cursor.to_string()))
}

pub(super) fn validate_turn(turn: &Value) -> Result<(&str, &str)> {
    let id = turn
        .get("id")
        .and_then(Value::as_str)
        .context("Codex turn page contains a turn without an ID")?;
    let status = turn
        .get("status")
        .and_then(Value::as_str)
        .context("Codex turn page contains a turn without a status")?;
    ensure!(
        matches!(
            status,
            "completed" | "interrupted" | "failed" | "inProgress"
        ),
        "Codex turn page contains an unsupported status `{status}`"
    );
    Ok((id, status))
}

pub(super) fn validate_item_entry(entry: &Value) -> Result<()> {
    ensure!(
        entry.get("turnId").and_then(Value::as_str).is_some(),
        "Codex item page contains an item without a turn ID"
    );
    let item = entry
        .get("item")
        .and_then(Value::as_object)
        .context("Codex item page contains an item without an object")?;
    ensure!(
        item.get("type").and_then(Value::as_str).is_some(),
        "Codex item page contains an item without a type"
    );
    Ok(())
}

pub(super) fn last_assistant_message_is_reflected(
    items: &[Value],
    turn_id: &str,
    expected: &str,
) -> Result<bool> {
    let mut last_message = None;
    for entry in items.iter().filter(|entry| {
        entry.get("turnId").and_then(Value::as_str) == Some(turn_id)
            && entry.pointer("/item/type").and_then(Value::as_str) == Some("agentMessage")
    }) {
        last_message = Some(
            entry
                .pointer("/item/text")
                .and_then(Value::as_str)
                .context("Codex assistant message item has no text")?,
        );
    }
    Ok(last_message == Some(expected))
}

pub(super) fn validate_item_turns(items: &[Value], turns: &[Value]) -> Result<()> {
    let turn_ids = turns
        .iter()
        .map(|turn| validate_turn(turn).map(|(id, _)| id.to_string()))
        .collect::<Result<HashSet<_>>>()?;
    for entry in items {
        let turn_id = entry
            .get("turnId")
            .and_then(Value::as_str)
            .context("Codex item page contains an item without a turn ID")?;
        ensure!(
            turn_ids.contains(turn_id),
            "Codex item page references a turn absent from thread/turns/list"
        );
    }
    Ok(())
}

pub(super) fn sanitize_nested_child_states(
    value: &mut Value,
    latest_states: &mut BTreeMap<String, String>,
) -> Result<()> {
    match value {
        Value::Array(values) => {
            for value in values {
                sanitize_nested_child_states(value, latest_states)?;
            }
        }
        Value::Object(object) => {
            if object.get("type").and_then(Value::as_str) == Some("collabAgentToolCall")
                && let Some(states) = object.get_mut("agentsStates")
            {
                let states = states
                    .as_object_mut()
                    .context("Codex collaboration item has invalid agentsStates")?;
                for (thread_id, state) in states {
                    let state = state
                        .as_object_mut()
                        .context("Codex collaboration item has an invalid child state")?;
                    let status = state
                        .get("status")
                        .and_then(Value::as_str)
                        .context("Codex child state has no status")?;
                    latest_states.insert(thread_id.clone(), status.to_string());
                    state.remove("message");
                }
            }
            for child in object.values_mut() {
                sanitize_nested_child_states(child, latest_states)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn add_history_bytes(total: &mut usize, value: &Value) -> Result<()> {
    let value_bytes = serde_json::to_vec(value)
        .context("serialize Codex history page")?
        .len();
    *total = total
        .checked_add(value_bytes)
        .context("Codex history size overflowed")?;
    ensure!(
        *total <= MAX_HISTORY_BYTES,
        "Codex history exceeds the {MAX_HISTORY_BYTES}-byte history limit"
    );
    Ok(())
}

pub(super) fn is_terminal_turn_status(status: &str) -> bool {
    matches!(status, "completed" | "interrupted" | "failed")
}
