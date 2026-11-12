use chrono::{DateTime, Utc};
use serde_json::Value;

use super::super::super::{ChatItem, UserContentPart};
use super::state::ToolUse;

#[derive(Clone)]
pub(super) struct TimedClaudeEvent {
    pub(super) timestamp: Option<DateTime<Utc>>,
    pub(super) effects: Vec<ClaudeEffect>,
}

#[derive(Clone)]
pub(super) enum ClaudeEffect {
    FlushTools,
    AssistantText {
        text: String,
    },
    Inbound {
        record: Value,
        text: String,
        images: Vec<UserContentPart>,
        use_user_marker: bool,
        simplify_slash_command: bool,
    },
    ToolAction {
        name: String,
        input: Option<Value>,
        failed: bool,
    },
    PeerNotification {
        tool_use: ToolUse,
        text: String,
        failed: bool,
        timestamp: Option<DateTime<Utc>>,
    },
    Question {
        id: Option<String>,
        item: ChatItem,
    },
    QuestionAnswers {
        id: String,
        answers: Vec<(String, String)>,
    },
}
