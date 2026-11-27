use std::collections::HashSet;

use serde_json::Value;

use super::super::transcript_branch::BranchMembership;
use super::ClaudeParserState;

#[derive(Clone, Default)]
pub(super) struct ClaudeBranchTracking {
    tool_use_assistant_ids: HashSet<usize>,
    side_branch_tool_use_assistant_ids: HashSet<usize>,
    valid_tool_result_ids: HashSet<usize>,
    compact_boundary_branch_ids: HashSet<usize>,
}

pub(super) struct ActiveRecordFlags {
    pub(super) pending_tool_result_on_active_branch: bool,
    pub(super) is_tool_result_only: bool,
    pub(super) is_tool_use_assistant: bool,
    pub(super) is_task_notification: bool,
}

impl ClaudeBranchTracking {
    pub(super) fn is_side_branch_continuation(
        &self,
        is_assistant: bool,
        is_tool_use_assistant: bool,
        parent_id: Option<usize>,
        parent_membership: Option<BranchMembership>,
    ) -> bool {
        match parent_membership {
            Some(BranchMembership::ReachesActive) => {
                is_assistant && parent_id.is_some_and(|id| self.valid_tool_result_ids.contains(&id))
            }
            Some(BranchMembership::Active) => {
                is_tool_use_assistant
                    && parent_id
                        .is_some_and(|id| self.side_branch_tool_use_assistant_ids.contains(&id))
            }
            _ => false,
        }
    }

    pub(super) fn observe_active_record(
        &mut self,
        record: &Value,
        uuid_id: Option<usize>,
        parent_id: Option<usize>,
        flags: ActiveRecordFlags,
    ) {
        if !flags.is_tool_result_only
            && (record.get("type").and_then(Value::as_str) == Some("user")
                || flags.is_task_notification)
        {
            self.reset_side_branch_continuations();
        }

        let Some(uuid_id) = uuid_id else {
            return;
        };

        if flags.is_tool_use_assistant {
            self.tool_use_assistant_ids.insert(uuid_id);
            self.side_branch_tool_use_assistant_ids.insert(uuid_id);
        }
        if flags.pending_tool_result_on_active_branch {
            self.valid_tool_result_ids.insert(uuid_id);
        }
        let extends_compact_boundary_branch = self.has_compact_boundary()
            && parent_id.is_some_and(|parent| self.compact_boundary_branch_ids.contains(&parent));
        let is_compact_boundary = record.get("type").and_then(Value::as_str) == Some("system")
            && record.get("subtype").and_then(Value::as_str) == Some("compact_boundary");
        if extends_compact_boundary_branch || is_compact_boundary {
            self.compact_boundary_branch_ids.insert(uuid_id);
        }
    }

    pub(super) fn is_compact_boundary_branch(&self, uuid_id: Option<usize>) -> bool {
        uuid_id.is_some_and(|uuid_id| self.compact_boundary_branch_ids.contains(&uuid_id))
    }

    pub(super) fn has_compact_boundary(&self) -> bool {
        !self.compact_boundary_branch_ids.is_empty()
    }

    pub(super) fn is_valid_tool_result_id(&self, uuid_id: usize) -> bool {
        self.valid_tool_result_ids.contains(&uuid_id)
    }

    pub(super) fn is_tool_use_assistant_id(&self, uuid_id: usize) -> bool {
        self.tool_use_assistant_ids.contains(&uuid_id)
    }

    fn reset_side_branch_continuations(&mut self) {
        self.side_branch_tool_use_assistant_ids.clear();
        self.valid_tool_result_ids.clear();
    }
}

impl ClaudeParserState {
    pub(in crate::commands::agent::watch_web::messages::parser::claude) fn is_valid_tool_result_uuid(
        &self,
        uuid: &str,
    ) -> bool {
        self.transcript_nodes
            .get(uuid)
            .is_some_and(|node| self.branch_tracking.is_valid_tool_result_id(node.uuid_id))
    }
}
