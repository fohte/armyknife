use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::super::transcript_branch::BranchMembership;
use super::ClaudeParserState;
use super::branch::has_tool_use_block;

#[derive(Clone, Default)]
pub(super) struct ClaudeBranchTracking {
    tool_use_assistant_uuids: HashSet<String>,
    side_branch_tool_use_assistant_uuids: HashSet<String>,
    valid_tool_result_uuids: HashSet<String>,
    side_branch_assistant_uuids: HashSet<String>,
    compact_boundary_branch_uuids: HashSet<String>,
    tool_result_owner_uuids: HashMap<String, HashSet<String>>,
}

impl ClaudeBranchTracking {
    pub(super) fn is_side_branch_continuation(
        &self,
        record: &Value,
        parent_uuid: Option<&str>,
        parent_membership: Option<BranchMembership>,
    ) -> bool {
        match parent_membership {
            Some(BranchMembership::ReachesActive) => {
                record.get("type").and_then(Value::as_str) == Some("assistant")
                    && parent_uuid.is_some_and(|uuid| self.valid_tool_result_uuids.contains(uuid))
            }
            Some(BranchMembership::Active) => {
                has_tool_use_block(record)
                    && parent_uuid.is_some_and(|uuid| {
                        self.side_branch_tool_use_assistant_uuids.contains(uuid)
                    })
            }
            _ => false,
        }
    }

    pub(super) fn observe_active_record(
        &mut self,
        record: &Value,
        uuid: Option<&str>,
        parent_uuid: Option<&str>,
        pending_tool_result_on_active_branch: bool,
        side_branch_continuation: bool,
        is_task_notification: bool,
    ) {
        if !super::super::user_input::is_tool_result_only_user_record(record)
            && (record.get("type").and_then(Value::as_str) == Some("user") || is_task_notification)
        {
            self.reset_side_branch_continuations();
        }

        let Some(uuid) = uuid else {
            return;
        };

        if has_tool_use_block(record) {
            self.tool_use_assistant_uuids.insert(uuid.to_owned());
            self.side_branch_tool_use_assistant_uuids
                .insert(uuid.to_owned());
        }
        if pending_tool_result_on_active_branch {
            self.valid_tool_result_uuids.insert(uuid.to_owned());
        }
        if side_branch_continuation
            && record.get("type").and_then(Value::as_str) == Some("assistant")
        {
            self.side_branch_assistant_uuids.insert(uuid.to_owned());
        }

        let extends_compact_boundary_branch =
            parent_uuid.is_some_and(|parent| self.compact_boundary_branch_uuids.contains(parent));
        let is_compact_boundary = record.get("type").and_then(Value::as_str) == Some("system")
            && record.get("subtype").and_then(Value::as_str) == Some("compact_boundary");
        if extends_compact_boundary_branch || is_compact_boundary {
            self.compact_boundary_branch_uuids.insert(uuid.to_owned());
        }
    }

    pub(super) fn is_compact_boundary_branch(&self, uuid: Option<&str>) -> bool {
        uuid.is_some_and(|uuid| self.compact_boundary_branch_uuids.contains(uuid))
    }

    pub(super) fn is_valid_tool_result_uuid(&self, uuid: &str) -> bool {
        self.valid_tool_result_uuids.contains(uuid)
    }

    pub(super) fn is_tool_use_assistant_uuid(&self, uuid: &str) -> bool {
        self.tool_use_assistant_uuids.contains(uuid)
    }

    pub(super) fn is_side_branch_assistant_uuid(&self, uuid: &str) -> bool {
        self.side_branch_assistant_uuids.contains(uuid)
    }

    pub(super) fn is_side_branch_tool_use_assistant_uuid(&self, uuid: &str) -> bool {
        self.side_branch_tool_use_assistant_uuids.contains(uuid)
    }

    pub(super) fn observe_tool_result_owner_uuids(
        &mut self,
        uuid: &str,
        owner_uuids: HashSet<String>,
    ) {
        self.tool_result_owner_uuids
            .insert(uuid.to_owned(), owner_uuids);
    }

    pub(super) fn tool_result_owner_uuids(&self, uuid: &str) -> Option<&HashSet<String>> {
        self.tool_result_owner_uuids.get(uuid)
    }

    fn reset_side_branch_continuations(&mut self) {
        // Tool ownership remains historical for result matching; only branch continuation expires.
        self.side_branch_tool_use_assistant_uuids.clear();
        self.valid_tool_result_uuids.clear();
        self.side_branch_assistant_uuids.clear();
        self.tool_result_owner_uuids.clear();
    }
}

impl ClaudeParserState {
    pub(in crate::commands::agent::watch_web::messages::parser::claude) fn is_valid_tool_result_uuid(
        &self,
        uuid: &str,
    ) -> bool {
        self.branch_tracking.is_valid_tool_result_uuid(uuid)
    }
}
