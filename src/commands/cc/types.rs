use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::error::CcError;

/// Tmux user option name for storing Claude Code session ID.
/// User options in tmux are prefixed with '@' and persist until explicitly unset.
/// Uses a descriptive name to avoid conflicts with other potential armyknife options.
pub const TMUX_SESSION_OPTION: &str = "@armyknife-last-claude-code-session-id";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub session_id: String,
    pub cwd: PathBuf,
    pub transcript_path: Option<PathBuf>,
    /// TTY device path (legacy field, not used for session lifecycle detection).
    #[serde(default)]
    pub tty: Option<String>,
    pub tmux_info: Option<TmuxInfo>,
    pub status: SessionStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_message: Option<String>,
    /// Currently executing tool name (e.g., "Bash", "Read", "Edit")
    #[serde(default)]
    pub current_tool: Option<String>,
    /// Short title for session identification (set via env var or auto-generated)
    #[serde(default)]
    pub label: Option<String>,
    /// Ancestor session IDs from root to immediate parent.
    /// Used to build tree view: if intermediate sessions are deleted,
    /// child sessions can still find their nearest living ancestor.
    #[serde(default)]
    pub ancestor_session_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TmuxInfo {
    pub session_name: String,
    pub window_name: String,
    pub window_index: u32,
    pub pane_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Running,
    WaitingInput,
    Stopped,
    /// Session has ended (Ctrl+D / /exit). Kept on disk so that `claude -c`
    /// resume can restore label and ancestor chain. Garbage-collected after
    /// a retention period by `cleanup_stale_sessions`.
    Ended,
}

impl SessionStatus {
    pub fn display_symbol(&self) -> &'static str {
        match self {
            Self::Running => "●",
            Self::WaitingInput => "◐",
            Self::Stopped | Self::Ended => "○",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::WaitingInput => "waiting",
            Self::Stopped => "stopped",
            Self::Ended => "ended",
        }
    }
}

/// Common fields present in all hook events.
#[derive(Debug, Deserialize)]
pub struct HookInput {
    pub session_id: String,
    pub cwd: PathBuf,
    #[serde(default)]
    pub transcript_path: Option<PathBuf>,

    // SessionStart event fields
    /// Source of the session start event: "startup" (new session) or "resume" (session restore).
    /// Used to skip "startup" events on `claude -c` which create unwanted empty sessions.
    #[serde(default)]
    pub source: Option<String>,

    // Notification event fields
    #[serde(default)]
    pub notification_type: Option<String>,

    // UserPromptSubmit event fields
    /// User's submitted prompt text (available in UserPromptSubmit events).
    /// Used for auto-generating session labels without reading from transcript files,
    /// which may not be written yet when the hook fires.
    #[serde(default)]
    pub prompt: Option<String>,

    // Pre-tool-use / Post-tool-use / PermissionRequest event fields
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_input: Option<ToolInput>,

    // Ignore other fields from Claude Code hooks
    #[serde(flatten)]
    _extra: serde_json::Value,
}

/// Tool input data from pre-tool-use events.
#[derive(Debug, Deserialize)]
pub struct ToolInput {
    /// Command for Bash tool
    pub command: Option<String>,
    /// File path for Read/Write/Edit tools
    pub file_path: Option<String>,
    /// Pattern for Grep/Glob tools
    pub pattern: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    SessionStart,
    UserPromptSubmit,
    PreToolUse,
    PostToolUse,
    PermissionRequest,
    Notification,
    Stop,
    SessionEnd,
}

impl HookEvent {
    pub fn from_str(s: &str) -> Result<Self> {
        match s {
            "session-start" => Ok(Self::SessionStart),
            "user-prompt-submit" => Ok(Self::UserPromptSubmit),
            "pre-tool-use" => Ok(Self::PreToolUse),
            "post-tool-use" => Ok(Self::PostToolUse),
            "permission-request" => Ok(Self::PermissionRequest),
            "notification" => Ok(Self::Notification),
            "stop" => Ok(Self::Stop),
            "session-end" => Ok(Self::SessionEnd),
            _ => Err(CcError::UnknownHookEvent(s.to_string()).into()),
        }
    }
}
