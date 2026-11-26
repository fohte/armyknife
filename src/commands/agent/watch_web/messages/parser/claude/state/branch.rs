use serde_json::Value;

use super::super::transcript_branch::{BranchMembership, is_inactive_branch};
use super::super::user_input::content_blocks;
use super::ClaudeParserState;

impl ClaudeParserState {
    pub(super) fn pending_tool_result_on_active_branch(
        &mut self,
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
            }),
            Some(BranchMembership::Unknown) => false,
            Some(BranchMembership::Disconnected | BranchMembership::Visiting) => false,
            None => true,
        }
    }

    pub(super) fn is_queued_peer_delivery(&mut self, record: &Value, uuid: Option<&str>) -> bool {
        let Some(uuid) = uuid.filter(|_| is_queued_peer_delivery_record(record)) else {
            return false;
        };
        let child_uuid = self.transcript_nodes.iter().find_map(|(child_uuid, node)| {
            (node.parent_uuid.as_deref() == Some(uuid)).then(|| child_uuid.clone())
        });
        child_uuid
            .is_some_and(|child_uuid| is_inactive_branch(&child_uuid, &mut self.transcript_nodes))
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
