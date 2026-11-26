use std::collections::HashSet;

use serde_json::Value;

use super::super::transcript_branch::BranchMembership;
use super::branch::has_tool_use_block;

#[derive(Clone, Default)]
pub(super) struct ClaudeBranchTracking {
    tool_use_assistant_uuids: HashSet<String>,
    valid_tool_result_uuids: HashSet<String>,
    compact_boundary_branch_uuids: HashSet<String>,
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
                    && parent_uuid.is_some_and(|uuid| self.tool_use_assistant_uuids.contains(uuid))
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
    ) {
        let Some(uuid) = uuid else {
            return;
        };

        if has_tool_use_block(record) {
            self.tool_use_assistant_uuids.insert(uuid.to_owned());
        }
        if pending_tool_result_on_active_branch {
            self.valid_tool_result_uuids.insert(uuid.to_owned());
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
}
