use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::commands::agent::store::PermissionRequestData;

use self::branch_tracking::ClaudeBranchTracking;
use super::super::super::SkillSuggestion;
use super::super::queue::QueueReplayEvent;
use super::super::record_timestamp;
use super::background_tasks::{BackgroundTaskTracker, task_notification_text};
use super::events::TimedClaudeEvent;
use super::transcript_branch::{BranchMembership, TranscriptNode, is_inactive_branch};
use super::user_input::is_tool_result_only_user_record;

mod branch;
mod branch_tracking;
mod effects;
mod queued_delivery;
mod tool_results;

pub(super) use self::effects::tool_result_text;

pub(super) fn is_attachment_record(record: &Value) -> bool {
    record.get("type").and_then(Value::as_str) == Some("attachment")
}

#[derive(Clone)]
pub(super) struct ClaudeParserState {
    pub(super) transcript_nodes: BTreeMap<String, TranscriptNode>,
    pub(super) transcript_children: HashMap<String, Vec<String>>,
    pub(super) forward_parent_uuids: HashSet<String>,
    branch_tracking: ClaudeBranchTracking,
    pub(super) latest_uuid: Option<String>,
    pub(super) latest_record_uuid: Option<String>,
    pub(super) tool_result_parent: Option<String>,
    pub(super) events: Vec<TimedClaudeEvent>,
    pub(super) queue_events: Vec<QueueReplayEvent>,
    pub(super) tool_uses: Vec<ToolUse>,
    pub(super) question_uses: HashMap<String, Option<String>>,
    pub(super) background_tasks: BackgroundTaskTracker,
    pub(super) skills: Vec<SkillSuggestion>,
}

impl ClaudeParserState {
    pub(super) fn new() -> Self {
        Self {
            transcript_nodes: BTreeMap::new(),
            transcript_children: HashMap::new(),
            forward_parent_uuids: HashSet::new(),
            branch_tracking: ClaudeBranchTracking::default(),
            latest_uuid: None,
            latest_record_uuid: None,
            tool_result_parent: None,
            events: Vec::new(),
            queue_events: Vec::new(),
            tool_uses: Vec::new(),
            question_uses: HashMap::new(),
            background_tasks: BackgroundTaskTracker::default(),
            skills: Vec::new(),
        }
    }

    pub(super) fn process_record(&mut self, record: Value, now: DateTime<Utc>) {
        if let Some(attachment) = record.get("attachment")
            && attachment.get("type").and_then(Value::as_str) == Some("skill_listing")
        {
            let listing =
                super::super::skill_suggestions::parse_claude_listing(attachment.get("content"));
            let is_initial = attachment.get("isInitial").and_then(Value::as_bool) != Some(false);
            super::super::skill_suggestions::update_claude_listing(
                &mut self.skills,
                listing,
                is_initial,
            );
        }

        let task_notification = task_notification_text(&record);
        let is_tool_result_only = is_tool_result_only_user_record(&record);
        let parent_uuid = record_parent_uuid(&record);
        let pending_tool_result = is_tool_result_only
            && self.has_pending_tool_result_for_parent(&record, parent_uuid.as_deref());
        let uuid = record.get("uuid").and_then(Value::as_str);
        let pending_tool_result_on_active_branch = self.pending_tool_result_on_active_branch(
            &record,
            uuid,
            parent_uuid.as_deref(),
            pending_tool_result,
        );
        let parent_membership = parent_uuid.as_deref().and_then(|parent_uuid| {
            if self
                .transcript_nodes
                .get(parent_uuid)
                .is_some_and(|node| node.membership == BranchMembership::Unknown)
            {
                is_inactive_branch(parent_uuid, &mut self.transcript_nodes);
            }
            self.transcript_nodes
                .get(parent_uuid)
                .map(|node| node.membership)
        });
        let side_branch_continuation = self.branch_tracking.is_side_branch_continuation(
            &record,
            parent_uuid.as_deref(),
            parent_membership,
        );
        let queued_peer_delivery = self.is_queued_peer_delivery(&record, uuid);
        let mut active_record = uuid.is_none();
        if is_tool_result_only {
            active_record = pending_tool_result_on_active_branch;
        } else if queued_peer_delivery || side_branch_continuation {
            active_record = true;
            if let Some(uuid) = uuid {
                is_inactive_branch(uuid, &mut self.transcript_nodes);
            }
        } else if let Some(uuid) = uuid {
            let membership = self.transcript_nodes.get(uuid).map(|node| node.membership);
            let bypass_branch_check = task_notification.is_some();
            active_record = match membership {
                None => bypass_branch_check,
                Some(BranchMembership::Unknown) if !bypass_branch_check => {
                    !is_inactive_branch(uuid, &mut self.transcript_nodes)
                }
                Some(BranchMembership::Active) => true,
                Some(
                    BranchMembership::ReachesActive
                    | BranchMembership::Disconnected
                    | BranchMembership::Visiting,
                ) => false,
                Some(BranchMembership::Unknown) => true,
            };
        }

        if let Some(mut queue_event) = QueueReplayEvent::from_record(&record) {
            queue_event.rendered_in_transcript = active_record;
            self.queue_events.push(queue_event);
        }
        if !active_record {
            return;
        }

        self.mark_record_processed(
            uuid,
            is_tool_result_only && pending_tool_result_on_active_branch,
        );

        self.branch_tracking.observe_active_record(
            &record,
            uuid,
            parent_uuid.as_deref(),
            pending_tool_result_on_active_branch,
            task_notification.is_some(),
        );
        if is_tool_result_only {
            self.tool_result_parent = parent_uuid;
        } else if uuid.is_some() && !is_attachment_record(&record) {
            self.tool_result_parent = None;
        }

        let effects = self.record_effects(&record, task_notification.as_deref(), now);
        self.events.push(TimedClaudeEvent {
            timestamp: record_timestamp(&record),
            effects,
        });
    }
}

pub(super) fn record_parent_uuid(record: &Value) -> Option<String> {
    match record.get("parentUuid") {
        Some(Value::String(parent)) => Some(parent.clone()),
        Some(Value::Null) => match record.get("logicalParentUuid") {
            None | Some(Value::Null) => None,
            Some(Value::String(parent)) => Some(parent.clone()),
            _ => None,
        },
        _ => None,
    }
}

#[derive(Clone)]
pub(super) struct ToolUse {
    pub(super) id: String,
    pub(super) assistant_uuid: Option<String>,
    pub(super) name: String,
    pub(super) input: Option<Value>,
    pub(super) started_at: Option<DateTime<Utc>>,
}

impl ToolUse {
    pub(super) fn matches_permission_request(&self, permission: &PermissionRequestData) -> bool {
        permission.tool_name.as_deref() == Some(self.name.as_str())
            && self.input.as_ref() == Some(&permission.tool_input)
    }
}
