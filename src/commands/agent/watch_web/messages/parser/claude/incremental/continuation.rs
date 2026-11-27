use serde_json::Value;

use super::super::background_tasks::task_notification_text;
use super::super::state::{ClaudeParserState, record_parent_uuid};
use super::super::transcript_branch::BranchMembership;
use super::super::user_input::is_tool_result_only_user_record;

pub(super) fn is_active_branch_continuation(state: &ClaudeParserState, record: &Value) -> bool {
    let Some(uuid) = record.get("uuid").and_then(Value::as_str) else {
        return true;
    };
    if state.transcript_nodes.contains_key(uuid) {
        return false;
    }
    let parent_uuid = record_parent_uuid(record);
    let is_tool_result_only = is_tool_result_only_user_record(record);
    let is_task_notification = task_notification_text(record).is_some();
    if is_tool_result_only && parent_uuid.is_none() {
        return false;
    }
    let Some(parent_uuid) = parent_uuid else {
        return state.latest_uuid.is_none();
    };
    let parent_is_active = state
        .transcript_nodes
        .get(&parent_uuid)
        .is_some_and(|node| {
            matches!(
                node.membership,
                BranchMembership::Active | BranchMembership::ReachesActive
            )
        });
    if !parent_is_active {
        return false;
    }
    let parent_is_latest = state.latest_uuid.as_deref() == Some(parent_uuid.as_str());
    let parent_is_latest_record = state.latest_record_uuid.as_deref() == Some(parent_uuid.as_str());
    let parent_is_latest_inert_attachment =
        state
            .transcript_nodes
            .get(&parent_uuid)
            .is_some_and(|node| {
                node.is_inert_attachment
                    && (state.latest_uuid.as_deref() == node.parent_uuid.as_deref()
                        || state.latest_record_uuid.as_deref() == node.parent_uuid.as_deref())
            });
    let supported_parallel_result_parent =
        is_tool_result_only && state.is_supported_parallel_tool_result_append(&parent_uuid);
    let assistant_continues_tool_result_parent = record.get("type").and_then(Value::as_str)
        == Some("assistant")
        && state.tool_result_parent.as_deref() == Some(parent_uuid.as_str())
        && state.latest_uuid.as_deref().is_some_and(|uuid| {
            state.is_valid_tool_result_uuid(uuid)
                && state
                    .transcript_nodes
                    .get(uuid)
                    .and_then(|node| node.parent_uuid.as_deref())
                    .and_then(|parent_uuid| state.transcript_nodes.get(parent_uuid))
                    .is_some_and(|parent| parent.has_tool_use)
        });
    if !parent_is_latest
        && !parent_is_latest_record
        && !parent_is_latest_inert_attachment
        && !supported_parallel_result_parent
        && !assistant_continues_tool_result_parent
    {
        return false;
    }

    if is_tool_result_only {
        let latest_record_is_compact_boundary_branch =
            state.latest_record_is_compact_boundary_branch();
        let parent_is_compact_boundary_branch = state.is_compact_boundary_branch(&parent_uuid);
        let parent_reaches_active = state
            .transcript_nodes
            .get(&parent_uuid)
            .is_some_and(|node| node.membership == BranchMembership::ReachesActive);
        let latest_tip_is_tool_result = state
            .latest_uuid
            .as_deref()
            .is_some_and(|uuid| state.is_valid_tool_result_uuid(uuid));
        let continues_another_result_branch =
            state.is_valid_tool_result_uuid(&parent_uuid) && !parent_is_latest;
        let replaces_a_result_branch =
            parent_reaches_active && !parent_is_latest && latest_tip_is_tool_result;
        if continues_another_result_branch
            || replaces_a_result_branch
            || state.has_unprocessed_assistant_after_sibling_tool_result(&parent_uuid)
            || state.has_unstable_retained_descendant(&parent_uuid)
        {
            return false;
        }
        return (!latest_record_is_compact_boundary_branch || parent_is_compact_boundary_branch)
            && (state.latest_uuid.as_deref() == Some(parent_uuid.as_str())
                || supported_parallel_result_parent)
            && state.has_pending_tool_result_for_parent(record, Some(parent_uuid.as_str()));
    }
    if is_task_notification {
        return parent_is_latest_record;
    }
    if record.get("type").and_then(Value::as_str) == Some("assistant") {
        return parent_is_latest_record
            || parent_is_latest
            || parent_is_latest_inert_attachment
            || state.latest_uuid.as_deref() == Some(parent_uuid.as_str())
            || assistant_continues_tool_result_parent;
    }
    parent_is_latest_record
}
