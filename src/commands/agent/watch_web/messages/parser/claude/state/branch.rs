use serde_json::Value;

use super::super::transcript_branch::{BranchMembership, content_ids, is_inactive_branch};
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
        let result_ids = content_ids(record, "tool_result", &["tool_use_id"]);
        let owner_tool_use_ids = owner.tool_use_ids.as_deref()?;
        if owner_tool_use_ids.len() != 1 || result_ids.as_slice() != owner_tool_use_ids {
            return None;
        }
        let mut candidates = self
            .transcript_children
            .get(&owner_uuid)?
            .iter()
            .filter_map(|uuid| {
                let node = self.transcript_nodes.get(uuid)?;
                (node.has_tool_use
                    && node
                        .tool_use_ids
                        .as_deref()
                        .is_some_and(|tool_use_ids| tool_use_ids.len() == 1)
                    && node.parent_uuid.as_deref() == Some(owner_uuid.as_str())
                    && !node.tool_use_ids.as_deref().is_some_and(|tool_use_ids| {
                        self.has_tool_result_for_tool(uuid, &tool_use_ids[0])
                    }))
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
                                .as_deref()
                                .unwrap_or_default()
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
        let record_count = self
            .transcript_nodes
            .values()
            .map(|node| node.record_order)
            .max()
            .and_then(|order| usize::try_from(order).ok())
            .and_then(|order| order.checked_add(1))
            .unwrap_or_default();
        self.transcript_parent_child_counts = vec![0; record_count];
        self.has_transcript_forks = false;
        for (uuid, node) in &self.transcript_nodes {
            if let Some(parent_uuid) = &node.parent_uuid {
                let parent = self.transcript_nodes.get(parent_uuid);
                if let Some(parent) = parent {
                    let parent_order = parent.record_order as usize;
                    let child_count = &mut self.transcript_parent_child_counts[parent_order];
                    self.has_transcript_forks |= *child_count > 0;
                    *child_count = child_count.saturating_add(1).min(2);
                }
                if (self.forward_parent_uuids.contains(parent_uuid)
                    || parent.is_some_and(|parent| parent.has_tool_use))
                    && (node.has_tool_use || node.is_tool_result_only)
                {
                    self.transcript_children
                        .entry(parent_uuid.clone())
                        .or_default()
                        .push(uuid.clone());
                }
            }
        }
    }

    pub(in crate::commands::agent::watch_web::messages::parser::claude) fn index_transcript_child(
        &mut self,
        parent_uuid: Option<&str>,
        uuid: &str,
    ) {
        if let Some(node) = self.transcript_nodes.get(uuid) {
            let prior_children = self
                .transcript_children
                .get(uuid)
                .map_or(0, |children| children.len().min(2)) as u8;
            if prior_children > 0 {
                let order = node.record_order as usize;
                if self.transcript_parent_child_counts.len() <= order {
                    self.transcript_parent_child_counts.resize(order + 1, 0);
                }
                self.transcript_parent_child_counts[order] = prior_children;
                self.has_transcript_forks |= prior_children > 1;
            }
        }
        if let Some(parent_uuid) = parent_uuid {
            if !self.transcript_nodes.contains_key(parent_uuid) {
                self.forward_parent_uuids.insert(parent_uuid.to_owned());
            }
            let parent = self.transcript_nodes.get(parent_uuid);
            if let Some(parent) = parent {
                let parent_order = parent.record_order as usize;
                if self.transcript_parent_child_counts.len() <= parent_order {
                    self.transcript_parent_child_counts
                        .resize(parent_order + 1, 0);
                }
                let child_count = &mut self.transcript_parent_child_counts[parent_order];
                self.has_transcript_forks |= *child_count > 0;
                *child_count = child_count.saturating_add(1).min(2);
            }
            if (self.forward_parent_uuids.contains(parent_uuid)
                || parent.is_some_and(|parent| parent.has_tool_use))
                && self
                    .transcript_nodes
                    .get(uuid)
                    .is_some_and(|node| node.has_tool_use || node.is_tool_result_only)
            {
                self.transcript_children
                    .entry(parent_uuid.to_owned())
                    .or_default()
                    .push(uuid.to_owned());
            }
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
        if self.all_transcript_records_active
            && !self.has_transcript_forks
            && !self.branch_tracking.has_compact_boundary()
        {
            return true;
        }
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
                self.transcript_nodes.get(parent_uuid).is_some_and(|node| {
                    node.membership == BranchMembership::Active
                        || self.branch_tracking.is_tool_use_assistant_id(node.uuid_id)
                })
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
        let Some(owner) = self.transcript_nodes.get(owner_uuid) else {
            return false;
        };
        if !self.branch_tracking.is_tool_use_assistant_id(owner.uuid_id)
            || !self
                .branch_tracking
                .is_compact_boundary_branch(Some(owner.uuid_id))
            || owner.membership != BranchMembership::Active
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
