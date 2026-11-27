use serde_json::Value;

use super::super::transcript_branch::BranchMembership;
use super::super::user_input::content_blocks;
use super::ClaudeParserState;

impl ClaudeParserState {
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

    pub(in crate::commands::agent::watch_web::messages::parser::claude) fn has_pending_tool_result_for_owner(
        &self,
        record: &Value,
        owner_uuid: &str,
    ) -> bool {
        let Some(message) = record.get("message") else {
            return false;
        };
        let source_assistant_uuid = record
            .get("sourceToolAssistantUUID")
            .and_then(Value::as_str);
        if source_assistant_uuid.is_some_and(|source_uuid| source_uuid != owner_uuid) {
            return false;
        }
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
                    && state
                        .transcript_nodes
                        .get(parent_uuid)
                        .is_some_and(|node| node.membership == BranchMembership::Active)
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
                    .any(|block| is_tool_use_block(block))
            })
}

fn is_tool_use_block(block: &Value) -> bool {
    block.get("type").and_then(Value::as_str) == Some("tool_use")
}

pub(super) fn tool_result_blocks(message: &Value) -> impl Iterator<Item = &Value> {
    message
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
}
