use serde_json::Value;

use super::super::background_tasks::task_notification_text;
use super::super::state::{ClaudeParserState, is_attachment_record, record_parent_uuid};
use super::super::transcript_branch::BranchMembership;
use super::super::user_input::{content_blocks, is_tool_result_only_user_record};

#[derive(Clone, Default)]
pub(super) struct AppendHistory {
    records: Vec<Value>,
}

impl AppendHistory {
    pub(super) fn observe(&mut self, record: &Value) {
        self.records.push(record.clone());
        if self.records.len() > 4 {
            self.records.remove(0);
        }
    }

    fn tail(&self, count: usize) -> Option<&[Value]> {
        (self.records.len() >= count).then(|| &self.records[self.records.len() - count..])
    }

    fn last(&self) -> Option<&Value> {
        self.records.last()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AppendDecision {
    SimpleTail,
    ParallelResult,
}

pub(super) fn classify_append(
    state: &ClaudeParserState,
    history: &AppendHistory,
    record: &Value,
) -> Option<AppendDecision> {
    let uuid = record.get("uuid").and_then(Value::as_str)?;
    if state.transcript_nodes.contains_key(uuid)
        || task_notification_text(record).is_some()
        || is_attachment_record(record)
    {
        return None;
    }

    if is_tool_result_only_user_record(record)
        && (is_first_parallel_result(state, history, record)
            || is_final_parallel_result(state, history, record))
    {
        return Some(AppendDecision::ParallelResult);
    }

    is_simple_tail_append(state, history, record).then_some(AppendDecision::SimpleTail)
}

fn is_simple_tail_append(
    state: &ClaudeParserState,
    history: &AppendHistory,
    record: &Value,
) -> bool {
    let Some(parent_uuid) = record_parent_uuid(record) else {
        return history.last().is_none()
            && state.latest_uuid.is_none()
            && state.latest_record_uuid.is_none()
            && state.transcript_nodes.is_empty()
            && is_simple_record(record)
            && !is_tool_result_only_user_record(record);
    };
    let Some(previous_record) = history.last() else {
        return false;
    };
    let Some(previous_uuid) = previous_record.get("uuid").and_then(Value::as_str) else {
        return false;
    };
    if previous_uuid != parent_uuid
        || state.latest_uuid.as_deref() != Some(parent_uuid.as_str())
        || state.latest_record_uuid.as_deref() != Some(parent_uuid.as_str())
        || !is_simple_record(previous_record)
        || !is_simple_record(record)
    {
        return false;
    }
    let Some(parent) = state.transcript_nodes.get(&parent_uuid) else {
        return false;
    };
    parent.membership == BranchMembership::Active
        && state
            .transcript_children
            .get(&parent_uuid)
            .is_none_or(Vec::is_empty)
        && (!is_tool_result_only_user_record(record)
            || state.has_pending_tool_result_for_parent(record, Some(&parent_uuid)))
}

fn is_first_parallel_result(
    state: &ClaudeParserState,
    history: &AppendHistory,
    record: &Value,
) -> bool {
    let Some([first_call, second_call]) = history.tail(2) else {
        return false;
    };
    let Some(first_uuid) = first_call.get("uuid").and_then(Value::as_str) else {
        return false;
    };
    let Some(second_uuid) = second_call.get("uuid").and_then(Value::as_str) else {
        return false;
    };
    is_tool_use_assistant(first_call)
        && is_tool_use_assistant(second_call)
        && is_simple_record(record)
        && record_parent_uuid(second_call).as_deref() == Some(first_uuid)
        && record_parent_uuid(record).as_deref() == Some(first_uuid)
        && result_ids_belong_to_owner(record, first_call)
        && state.has_pending_tool_result_for_owner(record, first_uuid)
        && state
            .transcript_nodes
            .get(first_uuid)
            .is_some_and(|node| node.membership == BranchMembership::Active && node.has_tool_use)
        && state.transcript_nodes.get(second_uuid).is_some_and(|node| {
            node.parent_uuid.as_deref() == Some(first_uuid)
                && node.has_tool_use
                && matches!(
                    node.membership,
                    BranchMembership::Active | BranchMembership::ReachesActive
                )
        })
        && latest_is_current_or_pending_parent(state, second_uuid, first_uuid)
}

fn is_final_parallel_result(
    state: &ClaudeParserState,
    history: &AppendHistory,
    record: &Value,
) -> bool {
    let Some([first_call, second_call, first_result, followup]) = history.tail(4) else {
        return false;
    };
    let Some(first_uuid) = first_call.get("uuid").and_then(Value::as_str) else {
        return false;
    };
    let Some(second_uuid) = second_call.get("uuid").and_then(Value::as_str) else {
        return false;
    };
    let Some(result_uuid) = first_result.get("uuid").and_then(Value::as_str) else {
        return false;
    };
    let Some(followup_uuid) = followup.get("uuid").and_then(Value::as_str) else {
        return false;
    };
    is_tool_use_assistant(first_call)
        && is_tool_use_assistant(second_call)
        && is_tool_result_only_user_record(first_result)
        && is_simple_record(first_result)
        && is_simple_record(record)
        && is_assistant(followup)
        && record_parent_uuid(second_call).as_deref() == Some(first_uuid)
        && record_parent_uuid(first_result).as_deref() == Some(first_uuid)
        && record_parent_uuid(followup).as_deref() == Some(result_uuid)
        && record_parent_uuid(record).as_deref() == Some(second_uuid)
        && result_ids_belong_to_owner(first_result, first_call)
        && result_ids_belong_to_owner(record, second_call)
        && state.has_pending_tool_result_for_owner(record, second_uuid)
        && state.is_valid_tool_result_uuid(result_uuid)
        && state
            .transcript_nodes
            .get(first_uuid)
            .is_some_and(|node| node.membership == BranchMembership::Active && node.has_tool_use)
        && state.transcript_nodes.get(second_uuid).is_some_and(|node| {
            node.parent_uuid.as_deref() == Some(first_uuid)
                && node.has_tool_use
                && matches!(
                    node.membership,
                    BranchMembership::Active | BranchMembership::ReachesActive
                )
        })
        && state
            .transcript_nodes
            .get(followup_uuid)
            .is_some_and(|node| {
                node.parent_uuid.as_deref() == Some(result_uuid)
                    && matches!(
                        node.membership,
                        BranchMembership::Active | BranchMembership::ReachesActive
                    )
            })
        && latest_is_current_or_pending_parent(state, followup_uuid, second_uuid)
}

fn latest_is_current_or_pending_parent(
    state: &ClaudeParserState,
    expected_tip_uuid: &str,
    pending_parent_uuid: &str,
) -> bool {
    [
        state.latest_uuid.as_deref(),
        state.latest_record_uuid.as_deref(),
    ]
    .into_iter()
    .all(|latest| latest == Some(expected_tip_uuid) || latest == Some(pending_parent_uuid))
}

fn result_ids_belong_to_owner(result: &Value, owner: &Value) -> bool {
    let owner_ids = tool_use_ids(owner);
    let result_ids = tool_result_ids(result);
    !result_ids.is_empty() && result_ids.iter().all(|id| owner_ids.contains(id))
}

fn tool_use_ids(record: &Value) -> Vec<&str> {
    record
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
        .filter_map(|block| block.get("id").and_then(Value::as_str))
        .collect()
}

fn tool_result_ids(record: &Value) -> Vec<&str> {
    record
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
        .filter_map(|block| block.get("tool_use_id").and_then(Value::as_str))
        .collect()
}

fn is_simple_record(record: &Value) -> bool {
    let record_type = record.get("type").and_then(Value::as_str);
    matches!(record_type, Some("assistant" | "user"))
        && !is_attachment_record(record)
        && task_notification_text(record).is_none()
        && record.get("origin").is_none()
        && record.get("isMeta").and_then(Value::as_bool) != Some(true)
        && record.get("isSidechain").and_then(Value::as_bool) != Some(true)
        && record.get("isCompactSummary").and_then(Value::as_bool) != Some(true)
        && (record_type != Some("user") || record.get("message").is_some())
}

fn is_assistant(record: &Value) -> bool {
    record.get("type").and_then(Value::as_str) == Some("assistant")
        && record.get("origin").is_none()
        && record.get("isMeta").and_then(Value::as_bool) != Some(true)
        && record.get("isSidechain").and_then(Value::as_bool) != Some(true)
        && record.get("isCompactSummary").and_then(Value::as_bool) != Some(true)
}

fn is_tool_use_assistant(record: &Value) -> bool {
    is_assistant(record)
        && record
            .get("message")
            .and_then(|message| message.get("content"))
            .is_some_and(|content| {
                content_blocks(content)
                    .iter()
                    .any(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
            })
}
