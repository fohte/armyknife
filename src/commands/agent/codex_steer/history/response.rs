use anyhow::{Context, Result, ensure};
use serde_json::Value;

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

pub(super) fn is_terminal_turn_status(status: &str) -> bool {
    matches!(status, "completed" | "interrupted" | "failed")
}
