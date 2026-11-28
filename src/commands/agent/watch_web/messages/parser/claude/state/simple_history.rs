use chrono::{DateTime, Utc};
use serde_json::Value;

use super::super::super::queue::QueueReplayEvent;
use super::super::super::record_timestamp;
use super::super::background_tasks::task_notification_text;
use super::super::events::TimedClaudeEvent;
use super::super::user_input::is_tool_result_only_user_record;
use super::branch_tracking::ActiveRecordFlags;
use super::{ClaudeParserState, is_attachment_record, record_parent_uuid_ref};

pub(super) fn process(
    state: &mut ClaudeParserState,
    record: &Value,
    metadata: Option<(bool, bool, usize)>,
    now: DateTime<Utc>,
    pending_tool_result_override: Option<bool>,
) {
    let uuid = record.get("uuid").and_then(Value::as_str);
    let task_notification = metadata
        .is_none_or(|(is_task_notification, _, _)| is_task_notification)
        .then(|| task_notification_text(record))
        .flatten();
    let is_tool_result_only = metadata.map_or_else(
        || is_tool_result_only_user_record(record),
        |(_, is_tool_result_only, _)| is_tool_result_only,
    );
    let uuid_id = metadata.map(|(_, _, uuid_id)| uuid_id).or_else(|| {
        uuid.and_then(|uuid| state.transcript_nodes.get(uuid).map(|node| node.uuid_id))
    });
    let parent_uuid = is_tool_result_only
        .then(|| record_parent_uuid_ref(record))
        .flatten();
    let pending_tool_result = pending_tool_result_override.unwrap_or_else(|| {
        is_tool_result_only && state.has_pending_tool_result_for_parent(record, parent_uuid)
    });
    if pending_tool_result || task_notification.is_some() {
        let uuid_id = metadata.map(|(_, _, uuid_id)| uuid_id).or_else(|| {
            uuid.and_then(|uuid| state.transcript_nodes.get(uuid).map(|node| node.uuid_id))
        });
        let parent_id = parent_uuid
            .and_then(|parent_uuid| state.transcript_nodes.get(parent_uuid))
            .map(|node| node.uuid_id);
        state.branch_tracking.observe_active_record(
            record,
            uuid_id,
            parent_id,
            ActiveRecordFlags {
                pending_tool_result_on_active_branch: pending_tool_result,
                is_tool_result_only,
                is_tool_use_assistant: false,
                is_task_notification: task_notification.is_some(),
            },
        );
    }
    if let Some(mut queue_event) = QueueReplayEvent::from_record(record) {
        queue_event.rendered_in_transcript = true;
        state.queue_events.push(queue_event);
    }
    if is_tool_result_only {
        state.tool_result_parent = parent_uuid.map(str::to_owned);
    } else if uuid.is_some() && !is_attachment_record(record) {
        state.tool_result_parent = None;
    }

    let effects = state.record_effects(record, task_notification.as_deref(), now);
    state.events.push(TimedClaudeEvent {
        uuid_id,
        timestamp: record_timestamp(record),
        effects,
    });
}
