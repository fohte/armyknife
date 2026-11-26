use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;

use crate::commands::agent::proposal::{ProposalAnswer, ProposalOption};
use crate::commands::agent::store::{
    AgentDecision, PermissionDeniedResolution, ProposalResolution, TodoResolution,
};

use super::questions::QuestionPrompt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FileAttachmentOrigin {
    Transcript,
    Stored,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum ChatItem {
    User {
        text: String,
        #[serde(rename = "rewindId", skip_serializing_if = "Option::is_none")]
        rewind_id: Option<String>,
        #[serde(rename = "queuedPromptId", skip_serializing_if = "Option::is_none")]
        queued_prompt_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        from: Option<String>,
    },
    QueuedUser {
        content: Vec<UserContentPart>,
        #[serde(rename = "queuedPromptId", skip_serializing_if = "Option::is_none")]
        queued_prompt_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        from: Option<String>,
    },
    UserWithContent {
        content: Vec<UserContentPart>,
        #[serde(rename = "rewindId", skip_serializing_if = "Option::is_none")]
        rewind_id: Option<String>,
        #[serde(rename = "queuedPromptId", skip_serializing_if = "Option::is_none")]
        queued_prompt_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        from: Option<String>,
    },
    Peer {
        direction: PeerDirection,
        session_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_title: Option<String>,
        content: Vec<UserContentPart>,
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
        #[serde(skip_serializing_if = "is_false")]
        failed: bool,
    },
    SubagentReport {
        agent_id: String,
        agent_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        text: String,
    },
    Event {
        kind: EventKind,
        text: String,
        failed: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
    },
    FileAttachment {
        files: Vec<super::super::file_attachments::ChatFile>,
        caption: Option<String>,
        #[serde(skip)]
        origin: FileAttachmentOrigin,
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
    },
    Assistant {
        text: String,
    },
    Narration {
        text: String,
        #[serde(rename = "thinkingDurationMs", skip_serializing_if = "Option::is_none")]
        thinking_duration_ms: Option<u64>,
    },
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "retained for watch-web responses that use standalone thought items"
        )
    )]
    Thinking {
        text: String,
        #[serde(rename = "thinkingDurationMs", skip_serializing_if = "Option::is_none")]
        thinking_duration_ms: Option<u64>,
    },
    ToolSummary {
        summary: String,
        operations: Vec<ToolOperation>,
    },
    #[serde(rename = "tool_summary")]
    ToolSummaryWithThinking {
        summary: String,
        operations: Vec<ToolOperation>,
        items: Vec<ToolGroupItem>,
    },
    ToolInProgress {
        tool: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        target: Option<String>,
    },
    #[serde(rename = "tool_in_progress")]
    ToolInProgressWithThinking {
        tool: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        target: Option<String>,
        items: Vec<ToolGroupItem>,
    },
    Question {
        #[serde(rename = "requestId", skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        #[serde(skip_serializing_if = "is_false")]
        cancelled: bool,
        questions: Vec<QuestionPrompt>,
    },
    Proposal {
        seq: i64,
        title: String,
        summary: Option<String>,
        options: Vec<ProposalOption>,
        answer: Option<ProposalAnswer>,
        resolved: bool,
        resolution: Option<ProposalResolution>,
        comment: Option<String>,
    },
    Todo {
        seq: i64,
        title: String,
        detail: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        folded: bool,
        #[serde(rename = "addedFolded", default, skip_serializing_if = "is_false")]
        added_folded: bool,
        completed: bool,
        resolution: Option<TodoResolution>,
        comment: Option<String>,
    },
    Permission {
        #[serde(rename = "requestId", skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        tool: String,
        description: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        command: Option<String>,
    },
    #[serde(rename = "permission")]
    PermissionWithThinking {
        #[serde(rename = "requestId", skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        tool: String,
        description: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        command: Option<String>,
        items: Vec<ToolGroupItem>,
    },
    PermissionDenied {
        seq: i64,
        #[serde(rename = "toolUseId", skip_serializing_if = "Option::is_none")]
        tool_use_id: Option<String>,
        tool: String,
        #[serde(rename = "inputSummary")]
        input_summary: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        command: Option<String>,
        reason: Option<String>,
        retryable: bool,
        resolved: bool,
        resolution: Option<PermissionDeniedResolution>,
    },
    Decision {
        seq: i64,
        title: String,
        why: String,
        considered: Vec<String>,
        #[serde(rename = "easyToChange")]
        easy_to_change: String,
        review: Option<String>,
        #[serde(rename = "createdAt")]
        created_at: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum ToolGroupItem {
    Thinking {
        text: String,
        #[serde(rename = "thinkingDurationMs", skip_serializing_if = "Option::is_none")]
        thinking_duration_ms: Option<u64>,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        tool: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        target: Option<String>,
        failed: bool,
    },
}

fn is_false(value: &bool) -> bool {
    !value
}

impl From<AgentDecision> for ChatItem {
    fn from(decision: AgentDecision) -> Self {
        Self::Decision {
            seq: decision.seq,
            title: decision.title,
            why: decision.why,
            considered: decision.considered,
            easy_to_change: decision.easy_to_change,
            review: decision.review,
            created_at: decision.created_at,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum PeerDirection {
    From,
    To,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum EventKind {
    Armyknife,
    CompactBoundary,
    System,
    Hook,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ConversationResponse {
    pub(super) messages: Vec<ChatItem>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) message_image_sources: BTreeMap<usize, BTreeMap<String, Option<String>>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) confirmed_queued_prompt_ids: Vec<String>,
    pub(super) background_tasks: Vec<BackgroundTask>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) skills: Vec<SkillSuggestion>,
    #[serde(skip)]
    pub(super) skill_suggestions_from_transcript: bool,
    #[serde(skip)]
    pub(super) skill_suggestions_are_fallback: bool,
    pub(super) start: usize,
    pub(super) total: usize,
    pub(super) decisions: Vec<ChatItem>,
    pub(super) has_unanswered_proposal: bool,
    pub(super) active_question_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) stable_start: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) stable_end: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) stable_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) epoch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) older_cursor: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub(super) epoch_reset: bool,
    #[serde(skip)]
    pub(super) tail_window_start: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip)]
    pub(super) initial_epoch: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::commands::agent::watch_web) struct SkillSuggestion {
    pub(in crate::commands::agent::watch_web) name: String,
    pub(in crate::commands::agent::watch_web) description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::commands::agent::watch_web) struct BackgroundTask {
    pub(in crate::commands::agent::watch_web) id: String,
    pub(in crate::commands::agent::watch_web) description: String,
    pub(in crate::commands::agent::watch_web) elapsed_seconds: u64,
    pub(in crate::commands::agent::watch_web) kind: BackgroundTaskKind,
    #[serde(skip)]
    pub(in crate::commands::agent::watch_web) output_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(in crate::commands::agent::watch_web) agent_type: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::commands::agent::watch_web) enum BackgroundTaskKind {
    Subagent,
    BackgroundTask,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum UserContentPart {
    Text { text: String },
    Image { source: String, available: bool },
    File { name: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct ToolOperation {
    pub(super) tool: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) target: Option<String>,
    pub(super) failed: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct ParentSession {
    pub(super) session_id: String,
    pub(super) title: String,
}
