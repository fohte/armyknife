use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::commands::agent::store::PermissionRequestData;

use self::branch_tracking::{ActiveRecordFlags, ClaudeBranchTracking};
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
mod simple_history;
mod tool_results;

pub(super) use self::effects::tool_result_text;

pub(super) fn is_attachment_record(record: &Value) -> bool {
    record.get("type").and_then(Value::as_str) == Some("attachment")
}

#[derive(Clone)]
pub(super) struct ClaudeParserState {
    pub(super) transcript_nodes: HashMap<String, TranscriptNode>,
    pub(super) next_uuid_id: usize,
    pub(super) has_duplicate_uuids: bool,
    pub(super) transcript_children: HashMap<String, Vec<String>>,
    pub(super) transcript_parent_child_counts: Vec<u8>,
    pub(super) has_transcript_forks: bool,
    pub(super) has_compact_boundary_record: bool,
    pub(super) all_transcript_records_active: bool,
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
            transcript_nodes: HashMap::new(),
            next_uuid_id: 0,
            has_duplicate_uuids: false,
            transcript_children: HashMap::new(),
            transcript_parent_child_counts: Vec::new(),
            has_transcript_forks: false,
            has_compact_boundary_record: false,
            all_transcript_records_active: false,
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

    pub(super) fn process_record(&mut self, record: &Value, now: DateTime<Utc>) {
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

        let uuid = record.get("uuid").and_then(Value::as_str);
        let metadata = (!self.has_duplicate_uuids)
            .then(|| uuid.and_then(|uuid| self.transcript_nodes.get(uuid)))
            .flatten();
        let simple_active_history = self.all_transcript_records_active
            && !self.has_transcript_forks
            && self.forward_parent_uuids.is_empty()
            && (uuid.is_none() || metadata.is_some())
            && !self.branch_tracking.has_compact_boundary();
        if simple_active_history {
            let metadata = metadata.map(|node| {
                (
                    node.is_task_notification,
                    node.is_tool_result_only,
                    node.uuid_id,
                )
            });
            simple_history::process(self, record, metadata, now);
            return;
        }
        let task_notification = if metadata.is_none_or(|node| node.is_task_notification) {
            task_notification_text(record)
        } else {
            None
        };
        let is_tool_result_only = metadata.map_or_else(
            || is_tool_result_only_user_record(record),
            |node| node.is_tool_result_only,
        );
        let record_type = record.get("type").and_then(Value::as_str);
        let is_assistant = record_type == Some("assistant");
        let is_compact_boundary = record_type == Some("system")
            && record.get("subtype").and_then(Value::as_str) == Some("compact_boundary");
        let needs_parent_uuid = is_tool_result_only
            || (is_assistant && self.has_transcript_forks)
            || is_compact_boundary
            || self.branch_tracking.has_compact_boundary();
        let parent_uuid = needs_parent_uuid
            .then(|| record_parent_uuid_ref(record))
            .flatten();
        let pending_tool_result =
            is_tool_result_only && self.has_pending_tool_result_for_parent(record, parent_uuid);
        let is_tool_use_assistant = is_assistant
            && uuid.is_some()
            && metadata.map_or_else(
                || {
                    record
                        .get("message")
                        .and_then(|message| message.get("content"))
                        .and_then(Value::as_array)
                        .is_some_and(|blocks| {
                            blocks.iter().any(|block| {
                                block.get("type").and_then(Value::as_str) == Some("tool_use")
                            })
                        })
                },
                |node| node.has_tool_use,
            );
        let pending_tool_result_on_active_branch = pending_tool_result
            && (simple_active_history
                || self.pending_tool_result_on_active_branch(
                    record,
                    uuid,
                    parent_uuid,
                    pending_tool_result,
                ));
        let mut uuid_id = None;
        let active_record = if simple_active_history {
            if is_tool_result_only
                || record_type == Some("user")
                || is_tool_use_assistant
                || is_compact_boundary
                || task_notification.is_some()
            {
                uuid_id =
                    uuid.and_then(|uuid| self.transcript_nodes.get(uuid).map(|node| node.uuid_id));
            }
            true
        } else {
            let side_branch_continuation = if is_assistant && self.has_transcript_forks {
                let parent_node = parent_uuid.and_then(|parent_uuid| {
                    if self
                        .transcript_nodes
                        .get(parent_uuid)
                        .is_some_and(|node| node.membership == BranchMembership::Unknown)
                    {
                        is_inactive_branch(parent_uuid, &mut self.transcript_nodes);
                    }
                    self.transcript_nodes.get(parent_uuid)
                });
                self.branch_tracking.is_side_branch_continuation(
                    is_assistant,
                    is_tool_use_assistant,
                    parent_node.map(|node| node.uuid_id),
                    parent_node.map(|node| node.membership),
                )
            } else {
                false
            };
            let queued_peer_delivery = !self.forward_parent_uuids.is_empty()
                && uuid.is_some_and(|uuid| {
                    self.forward_parent_uuids.contains(uuid)
                        && self.is_queued_peer_delivery(record, Some(uuid))
                });
            let mut active_record = uuid.is_none();
            if queued_peer_delivery || side_branch_continuation {
                active_record = true;
                if let Some(uuid) = uuid {
                    uuid_id = self.transcript_nodes.get(uuid).map(|node| node.uuid_id);
                    is_inactive_branch(uuid, &mut self.transcript_nodes);
                }
            } else if let Some(uuid) = uuid {
                let node = self.transcript_nodes.get(uuid);
                uuid_id = node.map(|node| node.uuid_id);
                let membership = node.map(|node| node.membership);
                let bypass_branch_check = is_tool_result_only || task_notification.is_some();
                active_record = match membership {
                    None => bypass_branch_check,
                    Some(BranchMembership::Unknown) if !bypass_branch_check => {
                        !is_inactive_branch(uuid, &mut self.transcript_nodes)
                    }
                    Some(_) => true,
                };
            }
            active_record
        };

        if let Some(mut queue_event) = QueueReplayEvent::from_record(record) {
            queue_event.rendered_in_transcript = active_record;
            self.queue_events.push(queue_event);
        }
        if !active_record {
            return;
        }

        let needs_branch_tracking = self.has_transcript_forks
            || self.has_compact_boundary_record
            || self.branch_tracking.has_compact_boundary()
            || is_compact_boundary
            || pending_tool_result_on_active_branch;
        if needs_branch_tracking
            && (is_tool_result_only
                || record_type == Some("user")
                || is_tool_use_assistant
                || is_compact_boundary
                || task_notification.is_some())
        {
            let parent_id = self
                .branch_tracking
                .has_compact_boundary()
                .then(|| {
                    parent_uuid.and_then(|parent| {
                        self.transcript_nodes.get(parent).map(|node| node.uuid_id)
                    })
                })
                .flatten();
            self.branch_tracking.observe_active_record(
                record,
                uuid_id,
                parent_id,
                ActiveRecordFlags {
                    pending_tool_result_on_active_branch,
                    is_tool_result_only,
                    is_tool_use_assistant,
                    is_task_notification: task_notification.is_some(),
                },
            );
        }
        if is_tool_result_only {
            self.tool_result_parent = parent_uuid.map(str::to_owned);
        } else if uuid.is_some() && !is_attachment_record(record) {
            self.tool_result_parent = None;
        }

        let effects = self.record_effects(record, task_notification.as_deref(), now);
        self.events.push(TimedClaudeEvent {
            uuid_id,
            timestamp: record_timestamp(record),
            effects,
        });
    }
}

pub(super) fn record_parent_uuid(record: &Value) -> Option<String> {
    record_parent_uuid_ref(record).map(str::to_owned)
}

fn record_parent_uuid_ref(record: &Value) -> Option<&str> {
    match record.get("parentUuid") {
        Some(Value::String(parent)) => Some(parent),
        Some(Value::Null) => match record.get("logicalParentUuid") {
            None | Some(Value::Null) => None,
            Some(Value::String(parent)) => Some(parent),
            _ => None,
        },
        _ => None,
    }
}

#[derive(Clone)]
pub(super) struct ToolUse {
    pub(super) id: String,
    pub(super) assistant_uuid: Option<String>,
    pub(super) assistant_uuid_id: Option<usize>,
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
