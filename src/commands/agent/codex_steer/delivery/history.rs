use std::collections::{HashMap, HashSet};

use serde_json::Value;

#[derive(Default)]
pub(super) struct MatchingItemSnapshot {
    item_ids_by_turn: HashMap<String, HashSet<String>>,
    item_counts_by_turn: HashMap<String, usize>,
}

pub(super) fn matching_user_message_snapshot(
    thread: &Value,
    content: &str,
) -> MatchingItemSnapshot {
    let mut snapshot = MatchingItemSnapshot::default();
    let Some(turns) = thread.get("turns").and_then(Value::as_array) else {
        return snapshot;
    };
    for turn in turns {
        let Some(turn_id) = turn.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(items) = turn.get("items").and_then(Value::as_array) else {
            continue;
        };
        for item in items.iter().filter(|item| {
            is_user_message(item)
                && item.get("clientId").and_then(Value::as_str).is_none()
                && has_same_text_content(item, content)
        }) {
            *snapshot
                .item_counts_by_turn
                .entry(turn_id.to_string())
                .or_default() += 1;
            if let Some(item_id) = item.get("id").and_then(Value::as_str) {
                snapshot
                    .item_ids_by_turn
                    .entry(turn_id.to_string())
                    .or_default()
                    .insert(item_id.to_string());
            }
        }
    }
    snapshot
}

pub(super) fn has_recorded_message(
    thread: &Value,
    notifications: &[Value],
    turn_id: &str,
    client_user_message_id: &str,
    content: &str,
    snapshot: &MatchingItemSnapshot,
) -> bool {
    has_user_message(thread, turn_id, client_user_message_id, content, snapshot)
        || has_user_message_notification(
            notifications,
            turn_id,
            client_user_message_id,
            content,
            snapshot,
        )
}

pub(super) fn has_user_message(
    thread: &Value,
    turn_id: &str,
    client_user_message_id: &str,
    content: &str,
    snapshot: &MatchingItemSnapshot,
) -> bool {
    let Some(items) = thread
        .get("turns")
        .and_then(Value::as_array)
        .and_then(|turns| {
            turns
                .iter()
                .find(|turn| turn.get("id").and_then(Value::as_str) == Some(turn_id))
        })
        .and_then(|turn| turn.get("items"))
        .and_then(Value::as_array)
    else {
        return false;
    };
    if items.iter().any(|item| {
        is_user_message(item)
            && item.get("clientId").and_then(Value::as_str) == Some(client_user_message_id)
    }) {
        return true;
    }
    let current_matching_count = items
        .iter()
        .filter(|item| {
            is_user_message(item)
                && item.get("clientId").and_then(Value::as_str).is_none()
                && has_same_text_content(item, content)
        })
        .count();
    current_matching_count
        > snapshot
            .item_counts_by_turn
            .get(turn_id)
            .copied()
            .unwrap_or(0)
}

fn is_user_message(item: &Value) -> bool {
    item.get("type").and_then(Value::as_str) == Some("userMessage")
}

fn has_same_text_content(item: &Value, content: &str) -> bool {
    item.get("content")
        .and_then(Value::as_array)
        .is_some_and(|inputs| {
            inputs.len() == 1
                && inputs[0].get("type").and_then(Value::as_str) == Some("text")
                && inputs[0].get("text").and_then(Value::as_str) == Some(content)
        })
}

pub(super) fn has_user_message_notification(
    notifications: &[Value],
    turn_id: &str,
    client_user_message_id: &str,
    content: &str,
    snapshot: &MatchingItemSnapshot,
) -> bool {
    notifications.iter().any(|notification| {
        matches!(
            notification.get("method").and_then(Value::as_str),
            Some("item/started" | "item/completed")
        ) && notification
            .pointer("/params/turnId")
            .and_then(Value::as_str)
            == Some(turn_id)
            && notification.pointer("/params/item").is_some_and(|item| {
                is_matching_user_message_notification(
                    item,
                    turn_id,
                    client_user_message_id,
                    content,
                    snapshot,
                )
            })
    })
}

fn is_matching_user_message_notification(
    item: &Value,
    turn_id: &str,
    client_user_message_id: &str,
    content: &str,
    snapshot: &MatchingItemSnapshot,
) -> bool {
    if !is_user_message(item) {
        return false;
    }
    match item.get("clientId").and_then(Value::as_str) {
        Some(item_client_id) => item_client_id == client_user_message_id,
        None => {
            has_same_text_content(item, content)
                && item
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|item_id| {
                        !snapshot
                            .item_ids_by_turn
                            .get(turn_id)
                            .is_some_and(|item_ids| item_ids.contains(item_id))
                    })
        }
    }
}

pub(super) fn has_completed_turn_notification(notifications: &[Value], turn_id: &str) -> bool {
    notifications
        .iter()
        .any(|notification| completed_turn_id(notification) == Some(turn_id))
}

fn completed_turn_id(notification: &Value) -> Option<&str> {
    (notification.get("method").and_then(Value::as_str) == Some("turn/completed"))
        .then(|| {
            notification
                .pointer("/params/turn/id")
                .and_then(Value::as_str)
        })
        .flatten()
}

pub(super) fn turn_status<'a>(thread: &'a Value, turn_id: &str) -> Option<&'a str> {
    thread
        .get("turns")
        .and_then(Value::as_array)?
        .iter()
        .find(|turn| turn.get("id").and_then(Value::as_str) == Some(turn_id))?
        .get("status")
        .and_then(Value::as_str)
}

pub(super) fn is_terminal_turn_status(status: &str) -> bool {
    matches!(status, "completed" | "interrupted" | "failed")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{MatchingItemSnapshot, has_user_message};

    #[test]
    fn ignores_same_body_in_a_different_turn() {
        let thread = json!({
            "turns": [
                {
                    "id": "turn-b",
                    "items": [{
                        "type": "userMessage",
                        "content": [{"type": "text", "text": "sample prompt"}],
                    }],
                },
                {"id": "turn-a", "items": []},
            ],
        });

        assert!(!has_user_message(
            &thread,
            "turn-a",
            "message-a",
            "sample prompt",
            &MatchingItemSnapshot::default(),
        ));
    }
}
