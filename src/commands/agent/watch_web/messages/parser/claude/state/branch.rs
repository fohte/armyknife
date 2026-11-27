use std::collections::HashSet;

use serde_json::Value;

use super::super::transcript_branch::{BranchMembership, is_inactive_branch};
use super::super::user_input::content_blocks;
use super::ClaudeParserState;

impl ClaudeParserState {
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

    pub(super) fn is_queued_peer_delivery(&mut self, record: &Value, uuid: Option<&str>) -> bool {
        let Some(uuid) = uuid.filter(|_| is_queued_peer_delivery_record(record)) else {
            return false;
        };
        if !self.forward_parent_uuids.contains(uuid) {
            return false;
        }
        self.transcript_children.get(uuid).is_some_and(|children| {
            children
                .iter()
                .any(|child_uuid| is_inactive_branch(child_uuid, &mut self.transcript_nodes))
        })
    }

    pub(in crate::commands::agent::watch_web::messages::parser::claude) fn has_pending_tool_result_for_parent(
        &self,
        record: &Value,
        parent_uuid: Option<&str>,
    ) -> bool {
        let Some(message) = record.get("message") else {
            return false;
        };
        let source_assistant_uuid = record
            .get("sourceToolAssistantUUID")
            .and_then(Value::as_str);
        let mut blocks = tool_result_blocks(message).peekable();
        blocks.peek().is_some()
            && blocks.all(|block| {
                block
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| {
                        self.tool_uses.iter().any(|tool_use| {
                            tool_use.id == id
                                && tool_owner_matches(
                                    tool_use.assistant_uuid.as_deref(),
                                    parent_uuid,
                                    source_assistant_uuid,
                                    self,
                                )
                        }) || self.question_uses.get(id).is_some_and(|assistant_uuid| {
                            tool_owner_matches(
                                assistant_uuid.as_deref(),
                                parent_uuid,
                                source_assistant_uuid,
                                self,
                            )
                        })
                    })
            })
    }

    pub(in crate::commands::agent::watch_web::messages::parser::claude) fn is_supported_parallel_tool_result_append(
        &self,
        parent_uuid: &str,
    ) -> bool {
        let Some(parent) = self.transcript_nodes.get(parent_uuid) else {
            return false;
        };
        if !parent.has_tool_use {
            return false;
        }
        if !self
            .branch_tracking
            .is_side_branch_tool_use_assistant_uuid(parent_uuid)
        {
            return false;
        }

        if self.latest_uuid.as_deref() == Some(parent_uuid) {
            return true;
        }

        if self.latest_uuid.as_deref().is_some_and(|latest_uuid| {
            self.transcript_nodes
                .get(latest_uuid)
                .is_some_and(|latest| {
                    latest.parent_uuid.as_deref() == Some(parent_uuid) && latest.has_tool_use
                })
        }) {
            return true;
        }

        if self.latest_uuid.as_deref().is_some_and(|latest_uuid| {
            self.transcript_children
                .get(parent_uuid)
                .is_some_and(|children| {
                    children.iter().any(|child_uuid| {
                        child_uuid == latest_uuid
                            && self.branch_tracking.is_valid_tool_result_uuid(child_uuid)
                    })
                })
        }) {
            return true;
        }

        let Some(owner_uuid) = parent.parent_uuid.as_deref() else {
            return false;
        };
        let Some(latest_uuid) = self.latest_uuid.as_deref() else {
            return false;
        };
        let Some(latest) = self.transcript_nodes.get(latest_uuid) else {
            return false;
        };
        latest.is_assistant
            && !latest.has_tool_use
            && latest.parent_uuid.as_deref().is_some_and(|sibling_uuid| {
                sibling_uuid != parent_uuid
                    && self.branch_tracking.is_valid_tool_result_uuid(sibling_uuid)
                    && self
                        .transcript_nodes
                        .get(sibling_uuid)
                        .is_some_and(|sibling| sibling.parent_uuid.as_deref() == Some(owner_uuid))
            })
    }

    pub(in crate::commands::agent::watch_web::messages::parser::claude) fn has_unstable_retained_descendant(
        &self,
        parent_uuid: &str,
    ) -> bool {
        let mut descendants = self
            .transcript_children
            .get(parent_uuid)
            .cloned()
            .unwrap_or_default();
        while let Some(uuid) = descendants.pop() {
            let Some(node) = self.transcript_nodes.get(&uuid) else {
                continue;
            };
            if node.is_inert_attachment {
                if let Some(children) = self.transcript_children.get(&uuid) {
                    descendants.extend(children.iter().cloned());
                }
                continue;
            }
            if !matches!(
                node.membership,
                BranchMembership::Active | BranchMembership::ReachesActive
            ) {
                continue;
            }
            let is_tool_result = self.branch_tracking.is_valid_tool_result_uuid(&uuid);
            let is_side_branch_assistant =
                self.branch_tracking.is_side_branch_assistant_uuid(&uuid);
            if node.membership == BranchMembership::Active
                && !is_tool_result
                && !is_side_branch_assistant
                && !(node.has_tool_use && node.parent_uuid.as_deref() == Some(parent_uuid))
            {
                return true;
            }
            if is_side_branch_assistant
                && node.parent_uuid.as_deref() != Some(parent_uuid)
                && node.parent_uuid.as_deref().is_some_and(|parent_uuid| {
                    !self.branch_tracking.is_valid_tool_result_uuid(parent_uuid)
                })
            {
                return true;
            }
            if is_tool_result
                && self.transcript_children.get(&uuid).is_some_and(|children| {
                    children.iter().any(|child_uuid| {
                        self.transcript_nodes
                            .get(child_uuid)
                            .is_some_and(|node| node.membership == BranchMembership::Active)
                            && self.branch_tracking.is_valid_tool_result_uuid(child_uuid)
                    })
                })
            {
                return true;
            }
            if let Some(children) = self.transcript_children.get(&uuid) {
                descendants.extend(children.iter().cloned());
            }
        }
        false
    }

    pub(in crate::commands::agent::watch_web::messages::parser::claude) fn has_unprocessed_assistant_after_sibling_tool_result(
        &self,
        parent_uuid: &str,
    ) -> bool {
        let mut path_child_uuid = parent_uuid.to_owned();
        let mut visited = HashSet::new();
        while visited.insert(path_child_uuid.clone())
            && let Some(ancestor_uuid) = self
                .transcript_nodes
                .get(&path_child_uuid)
                .and_then(|node| node.parent_uuid.clone())
        {
            if let Some(siblings) = self.transcript_children.get(&ancestor_uuid) {
                for sibling_uuid in siblings
                    .iter()
                    .filter(|sibling_uuid| *sibling_uuid != &path_child_uuid)
                {
                    if self.sibling_branch_has_unstable_assistant_descendant(sibling_uuid) {
                        return true;
                    }
                }
            }
            path_child_uuid = ancestor_uuid;
        }
        false
    }

    fn sibling_branch_has_unstable_assistant_descendant(&self, branch_root: &str) -> bool {
        let mut visited = HashSet::new();
        let mut descendants = vec![(branch_root.to_owned(), false)];
        while let Some((uuid, has_result_ancestor)) = descendants.pop() {
            if !visited.insert(uuid.clone()) {
                continue;
            }
            let Some(node) = self.transcript_nodes.get(&uuid) else {
                continue;
            };
            if !matches!(
                node.membership,
                BranchMembership::Active | BranchMembership::ReachesActive
            ) {
                continue;
            }
            let is_valid_result = self.branch_tracking.is_valid_tool_result_uuid(&uuid);
            let has_valid_result_ancestor = has_result_ancestor || is_valid_result;
            let parent_is_valid_result = node.parent_uuid.as_deref().is_some_and(|parent_uuid| {
                self.branch_tracking.is_valid_tool_result_uuid(parent_uuid)
            });
            if is_valid_result
                && self.transcript_children.get(&uuid).is_some_and(|children| {
                    children.iter().any(|child_uuid| {
                        self.transcript_nodes
                            .get(child_uuid)
                            .is_some_and(|child| child.is_assistant && !child.processed)
                    })
                })
            {
                return true;
            }
            if node.is_assistant && node.processed && has_result_ancestor && !parent_is_valid_result
            {
                return true;
            }
            if let Some(children) = self.transcript_children.get(&uuid) {
                descendants.extend(
                    children
                        .iter()
                        .cloned()
                        .map(|child_uuid| (child_uuid, has_valid_result_ancestor)),
                );
            }
        }
        false
    }
}

fn is_queued_peer_delivery_record(record: &Value) -> bool {
    record.get("type").and_then(Value::as_str) == Some("user")
        && record.get("isMeta").and_then(Value::as_bool) == Some(true)
        && record
            .get("origin")
            .and_then(|origin| origin.get("kind"))
            .and_then(Value::as_str)
            == Some("peer")
        && record.get("message").is_some_and(|message| {
            let text = super::super::user_input::user_text(message);
            super::super::super::super::queued_prompt::id_from_content(&text).is_some()
        })
}

fn tool_owner_matches(
    tool_assistant_uuid: Option<&str>,
    parent_uuid: Option<&str>,
    source_assistant_uuid: Option<&str>,
    state: &ClaudeParserState,
) -> bool {
    tool_assistant_uuid.is_none_or(|tool_assistant_uuid| {
        (parent_uuid.is_none() && source_assistant_uuid.is_none())
            || parent_uuid == Some(tool_assistant_uuid)
            || source_assistant_uuid == Some(tool_assistant_uuid)
            || parent_uuid.is_some_and(|parent_uuid| {
                state.branch_tracking.is_valid_tool_result_uuid(parent_uuid)
                    && state.transcript_nodes.get(parent_uuid).is_some_and(|node| {
                        node.membership == super::super::transcript_branch::BranchMembership::Active
                    })
            })
    })
}

pub(super) fn has_tool_use_block(record: &Value) -> bool {
    record.get("type").and_then(Value::as_str) == Some("assistant")
        && record
            .get("message")
            .and_then(|message| message.get("content"))
            .is_some_and(|content| {
                content_blocks(content)
                    .iter()
                    .any(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
            })
}

pub(super) fn tool_result_blocks(message: &Value) -> impl Iterator<Item = &Value> {
    message
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
}
