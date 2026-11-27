use chrono::{DateTime, Utc};
use serde_json::Value;

use super::super::super::{ChatItem, UserContentPart};
use super::state::ToolUse;

#[derive(Clone)]
pub(super) struct TimedClaudeEvent {
    pub(super) uuid_id: Option<usize>,
    pub(super) timestamp: Option<DateTime<Utc>>,
    pub(super) effects: Vec<ClaudeEffect>,
}

#[derive(Clone)]
pub(super) enum ClaudeEffect {
    FlushTools,
    AssistantText {
        text: String,
    },
    Narration {
        text: String,
        duration_ms: Option<u64>,
    },
    Thinking {
        text: String,
        duration_ms: Option<u64>,
    },
    Inbound {
        record: Value,
        text: String,
        images: Vec<UserContentPart>,
        use_user_marker: bool,
        simplify_slash_command: bool,
        rewind_id: Option<String>,
    },
    ToolAction {
        tool_assistant_uuid_id: Option<usize>,
        name: String,
        input: Option<Value>,
        failed: bool,
    },
    SendUserFile {
        tool_use: ToolUse,
        failed: bool,
    },
    Question {
        id: Option<String>,
        item: ChatItem,
    },
    QuestionAnswers {
        id: String,
        answers: Vec<(String, String)>,
    },
    QuestionCancelled {
        id: String,
    },
}
