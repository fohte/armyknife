use serde_json::Value;

use super::super::transcript_branch::{BranchMembership, is_inactive_branch};
use super::ClaudeParserState;
use super::tool_results::tool_result_blocks;

impl ClaudeParserState {
    pub(in crate::commands::agent::watch_web::messages::parser::claude) fn parallel_pending_tool_use_uuid(
        &self,
        record: &Value,
    ) -> Option<String> {
        if !super::super::user_input::is_tool_result_only_user_record(record) {
            return None;
        }
        let owner_uuid = super::record_parent_uuid(record)?;
        let owner = self.transcript_nodes.get(&owner_uuid)?;
        let result_ids = record_tool_result_ids(record);
        if owner.tool_use_ids.len() != 1 || result_ids != owner.tool_use_ids {
            return None;
        }
        let mut candidates = self
            .transcript_children
            .get(&owner_uuid)?
            .iter()
            .filter_map(|uuid| {
                let node = self.transcript_nodes.get(uuid)?;
                (node.has_tool_use
                    && node.tool_use_ids.len() == 1
                    && node.parent_uuid.as_deref() == Some(owner_uuid.as_str())
                    && !self.has_tool_result_for_tool(uuid, &node.tool_use_ids[0]))
                .then(|| uuid.clone())
            });
        let candidate = candidates.next()?;
        candidates.next().is_none().then_some(candidate)
    }

    fn has_tool_result_for_tool(&self, assistant_uuid: &str, tool_use_id: &str) -> bool {
        self.transcript_children
            .get(assistant_uuid)
            .is_some_and(|children| {
                children.iter().any(|child_uuid| {
                    self.transcript_nodes.get(child_uuid).is_some_and(|child| {
                        child.is_tool_result_only
                            && child
                                .tool_result_ids
                                .iter()
                                .any(|result_id| result_id == tool_use_id)
                    })
                })
            })
    }

    pub(in crate::commands::agent::watch_web::messages::parser::claude) fn rebuild_transcript_children(
        &mut self,
    ) {
        self.transcript_children.clear();
        for (uuid, node) in &self.transcript_nodes {
            if let Some(parent_uuid) = &node.parent_uuid {
                self.transcript_children
                    .entry(parent_uuid.clone())
                    .or_default()
                    .push(uuid.clone());
            }
        }
    }

    pub(in crate::commands::agent::watch_web::messages::parser::claude) fn index_transcript_child(
        &mut self,
        parent_uuid: Option<&str>,
        uuid: &str,
    ) {
        if let Some(parent_uuid) = parent_uuid {
            if !self.transcript_nodes.contains_key(parent_uuid) {
                self.forward_parent_uuids.insert(parent_uuid.to_owned());
            }
            self.transcript_children
                .entry(parent_uuid.to_owned())
                .or_default()
                .push(uuid.to_owned());
        }
    }

    pub(super) fn mark_record_processed(
        &mut self,
        uuid: Option<&str>,
        was_valid_tool_result: bool,
    ) {
        if let Some(node) = uuid.and_then(|uuid| self.transcript_nodes.get_mut(uuid)) {
            node.processed = true;
            node.was_valid_tool_result |= was_valid_tool_result;
        }
    }

    pub(super) fn pending_tool_result_on_active_branch(
        &mut self,
        record: &Value,
        uuid: Option<&str>,
        parent_uuid: Option<&str>,
        pending_tool_result: bool,
    ) -> bool {
        if !pending_tool_result {
            return false;
        }
        let Some(uuid) = uuid else {
            return true;
        };
        if self
            .transcript_nodes
            .get(uuid)
            .is_some_and(|node| node.membership == BranchMembership::Unknown)
        {
            is_inactive_branch(uuid, &mut self.transcript_nodes);
        }
        match self.transcript_nodes.get(uuid).map(|node| node.membership) {
            Some(BranchMembership::Active) => true,
            Some(BranchMembership::ReachesActive) => parent_uuid.is_some_and(|parent_uuid| {
                self.transcript_nodes
                    .get(parent_uuid)
                    .is_some_and(|node| node.membership == BranchMembership::Active)
                    || self.branch_tracking.is_tool_use_assistant_uuid(parent_uuid)
            }),
            Some(BranchMembership::Unknown) => false,
            Some(BranchMembership::Disconnected) => {
                self.pending_tool_result_from_active_owner(record)
            }
            Some(BranchMembership::Visiting) => false,
            None => true,
        }
    }

    fn pending_tool_result_from_active_owner(&self, record: &Value) -> bool {
        let Some(owner_uuid) = record
            .get("sourceToolAssistantUUID")
            .and_then(Value::as_str)
        else {
            return false;
        };
        if !self.branch_tracking.is_tool_use_assistant_uuid(owner_uuid)
            || !self
                .branch_tracking
                .is_compact_boundary_branch(Some(owner_uuid))
            || !self
                .transcript_nodes
                .get(owner_uuid)
                .is_some_and(|node| node.membership == BranchMembership::Active)
        {
            return false;
        }
        let Some(message) = record.get("message") else {
            return false;
        };
        let mut blocks = tool_result_blocks(message).peekable();
        blocks.peek().is_some()
            && blocks.all(|block| {
                block
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| {
                        self.tool_uses.iter().any(|tool_use| {
                            tool_use.id == id
                                && tool_use.assistant_uuid.as_deref() == Some(owner_uuid)
                        }) || self.question_uses.get(id).is_some_and(|assistant_uuid| {
                            assistant_uuid.as_deref() == Some(owner_uuid)
                        })
                    })
            })
    }
}

fn record_tool_result_ids(record: &Value) -> Vec<String> {
    let Some(message) = record.get("message") else {
        return Vec::new();
    };
    tool_result_blocks(message)
        .filter_map(|block| block.get("tool_use_id").and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}
