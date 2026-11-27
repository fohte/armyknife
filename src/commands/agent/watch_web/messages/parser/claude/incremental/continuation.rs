use serde_json::Value;

use super::super::background_tasks::task_notification_text;
use super::super::state::{ClaudeParserState, is_attachment_record, record_parent_uuid};
use super::super::transcript_branch::BranchMembership;
use super::super::user_input::user_content;

#[derive(Clone, Default)]
pub(super) struct AppendHistory {
    records: Vec<AppendRecord>,
    initial_records: Vec<Value>,
}

impl AppendHistory {
    pub(super) fn observe_initial(&mut self, record: Value) {
        self.initial_records.push(record);
        if self.initial_records.len() > 4 {
            self.initial_records.remove(0);
        }
    }

    pub(super) fn finish_initial(&mut self) {
        self.records = std::mem::take(&mut self.initial_records)
            .into_iter()
            .map(|record| AppendRecord::from_record(&record))
            .collect();
    }

    pub(super) fn observe_append(&mut self, classified: ClassifiedAppend) {
        self.records.push(classified.record);
        if self.records.len() > 4 {
            self.records.remove(0);
        }
    }

    fn tail(&self, count: usize) -> Option<&[AppendRecord]> {
        (self.records.len() >= count).then(|| &self.records[self.records.len() - count..])
    }

    fn last(&self) -> Option<&AppendRecord> {
        self.records.last()
    }
}

#[derive(Clone, Default)]
struct AppendRecord {
    uuid: Option<String>,
    parent_uuid: Option<String>,
    is_simple: bool,
    is_assistant: bool,
    is_tool_use_assistant: bool,
    is_tool_result_only: bool,
    is_transparent_attachment: bool,
    is_task_notification: bool,
    tool_use_count: usize,
}

impl AppendRecord {
    fn from_record(record: &Value) -> Self {
        let record_type = record.get("type").and_then(Value::as_str);
        let is_user = record_type == Some("user");
        let mut has_user_content = false;
        let mut has_non_result_user_content = false;
        let mut tool_use_count = 0;
        if let Some(blocks) = record
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(Value::as_array)
        {
            for block in blocks {
                match block.get("type").and_then(Value::as_str) {
                    Some("tool_use") => {
                        tool_use_count += 1;
                    }
                    Some("tool_result") => {
                        if is_user {
                            has_user_content = true;
                        }
                    }
                    _ if is_user => {
                        has_user_content = true;
                        has_non_result_user_content = true;
                    }
                    _ => {}
                }
            }
        }
        let is_tool_result_only = is_user && has_user_content && !has_non_result_user_content;
        let is_task_notification = task_notification_text(record).is_some();
        let is_attachment = is_attachment_record(record);
        let has_supported_branch_metadata = has_supported_branch_metadata(record);
        let is_transparent_attachment = is_transparent_attachment(record, is_task_notification);
        let is_assistant = record_type == Some("assistant") && has_supported_branch_metadata;
        let is_simple = matches!(record_type, Some("assistant" | "user"))
            && !is_attachment
            && !is_task_notification
            && has_supported_branch_metadata
            && (record_type != Some("user") || record.get("message").is_some());
        Self {
            uuid: record
                .get("uuid")
                .and_then(Value::as_str)
                .map(str::to_owned),
            parent_uuid: record_parent_uuid(record),
            is_simple: is_simple || is_transparent_attachment,
            is_assistant,
            is_tool_use_assistant: is_assistant && tool_use_count > 0,
            is_tool_result_only,
            is_transparent_attachment,
            is_task_notification,
            tool_use_count,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AppendDecision {
    SimpleTail,
    ParallelResult,
}

pub(super) struct ClassifiedAppend {
    pub(super) decision: AppendDecision,
    record: AppendRecord,
}

impl ClassifiedAppend {
    pub(super) fn simple_node_flags(&self) -> (bool, bool, bool) {
        (
            self.record.is_tool_use_assistant,
            self.record.is_tool_result_only,
            self.record.is_task_notification,
        )
    }

    pub(super) fn extends_active_record_cache(&self) -> bool {
        self.record.is_tool_result_only || self.record.is_task_notification
    }
}

pub(super) fn classify_append(
    state: &ClaudeParserState,
    history: &AppendHistory,
    record: &Value,
) -> Option<ClassifiedAppend> {
    let candidate = AppendRecord::from_record(record);
    let uuid = candidate.uuid.as_deref()?;
    if state.transcript_nodes.contains_key(uuid)
        || candidate.is_task_notification
        || (is_attachment_record(record) && !candidate.is_transparent_attachment)
    {
        return None;
    }

    if candidate.is_tool_result_only
        && (is_first_parallel_result(state, history, record, &candidate)
            || is_final_parallel_result(state, history, record, &candidate))
    {
        return Some(ClassifiedAppend {
            decision: AppendDecision::ParallelResult,
            record: candidate,
        });
    }

    is_simple_tail_append(state, history, record, &candidate).then_some(ClassifiedAppend {
        decision: AppendDecision::SimpleTail,
        record: candidate,
    })
}

fn is_simple_tail_append(
    state: &ClaudeParserState,
    history: &AppendHistory,
    record: &Value,
    candidate: &AppendRecord,
) -> bool {
    let Some(parent_uuid) = candidate.parent_uuid.as_ref() else {
        return history.last().is_none()
            && state.latest_uuid.is_none()
            && state.latest_record_uuid.is_none()
            && state.transcript_nodes.is_empty()
            && candidate.is_simple
            && !candidate.is_tool_result_only;
    };
    let Some(previous_record) = history.last() else {
        return false;
    };
    let Some(previous_uuid) = previous_record.uuid.as_deref() else {
        return false;
    };
    if previous_uuid != parent_uuid
        || state.latest_record_uuid.as_deref() != Some(parent_uuid.as_str())
        || !previous_record.is_simple
        || !candidate.is_simple
    {
        return false;
    }
    if state.latest_uuid.as_deref() != Some(parent_uuid.as_str())
        && !previous_record.is_transparent_attachment
    {
        return false;
    }
    let Some(parent) = state.transcript_nodes.get(parent_uuid) else {
        return false;
    };
    parent.membership == BranchMembership::Active
        && state
            .transcript_parent_child_counts
            .get(parent.record_order as usize)
            .is_none_or(|child_count| *child_count == 0)
        && (!candidate.is_tool_result_only
            || state.has_pending_tool_result_for_parent(record, Some(parent_uuid.as_str())))
}

fn is_first_parallel_result(
    state: &ClaudeParserState,
    history: &AppendHistory,
    record: &Value,
    candidate: &AppendRecord,
) -> bool {
    let Some(records) = history.tail(2) else {
        return false;
    };
    let [first_call, second_call] = records else {
        return false;
    };
    let Some(first_uuid) = first_call.uuid.as_deref() else {
        return false;
    };
    let Some(second_uuid) = second_call.uuid.as_deref() else {
        return false;
    };
    first_call.is_tool_use_assistant
        && second_call.is_tool_use_assistant
        && first_call.tool_use_count == 1
        && second_call.tool_use_count == 1
        && candidate.is_simple
        && second_call.parent_uuid.as_deref() == Some(first_uuid)
        && candidate.parent_uuid.as_deref() == Some(first_uuid)
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
    candidate: &AppendRecord,
) -> bool {
    let Some(records) = history.tail(4) else {
        return false;
    };
    let [first_call, second_call, first_result, followup] = records else {
        return false;
    };
    let Some(first_uuid) = first_call.uuid.as_deref() else {
        return false;
    };
    let Some(second_uuid) = second_call.uuid.as_deref() else {
        return false;
    };
    let Some(result_uuid) = first_result.uuid.as_deref() else {
        return false;
    };
    let Some(followup_uuid) = followup.uuid.as_deref() else {
        return false;
    };
    first_call.is_tool_use_assistant
        && second_call.is_tool_use_assistant
        && first_call.tool_use_count == 1
        && second_call.tool_use_count == 1
        && first_result.is_tool_result_only
        && first_result.is_simple
        && candidate.is_simple
        && followup.is_assistant
        && second_call.parent_uuid.as_deref() == Some(first_uuid)
        && first_result.parent_uuid.as_deref() == Some(first_uuid)
        && followup.parent_uuid.as_deref() == Some(result_uuid)
        && candidate.parent_uuid.as_deref() == Some(second_uuid)
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

fn is_transparent_attachment(record: &Value, is_task_notification: bool) -> bool {
    if !is_attachment_record(record) {
        return false;
    }
    let Some(attachment) = record.get("attachment") else {
        return false;
    };
    match attachment.get("type").and_then(Value::as_str) {
        Some("hook_success") => true,
        Some("queued_command") if !is_task_notification => {
            let (text, images) = user_content(attachment.get("prompt"));
            text.is_empty() && images.is_empty()
        }
        _ => false,
    }
}

fn has_supported_branch_metadata(record: &Value) -> bool {
    let has_supported_origin = record.get("origin").is_none_or(|origin| {
        record.get("type").and_then(Value::as_str) == Some("user")
            && origin.get("kind").and_then(Value::as_str) == Some("human")
    });
    has_supported_origin
        && record.get("isMeta").and_then(Value::as_bool) != Some(true)
        && record.get("isSidechain").and_then(Value::as_bool) != Some(true)
        && record.get("isCompactSummary").and_then(Value::as_bool) != Some(true)
}
