use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::error::CcError;

/// Tmux user option name for storing Claude Code session ID.
/// User options in tmux are prefixed with '@' and persist until explicitly unset.
pub const TMUX_SESSION_OPTION: &str = "@armyknife-session-id";

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
}

impl SessionStatus {
    pub fn display_symbol(&self) -> &'static str {
        match self {
            Self::Running => "●",
            Self::WaitingInput => "◐",
            Self::Stopped => "○",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::WaitingInput => "waiting",
            Self::Stopped => "stopped",
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

    // Notification event fields
    #[serde(default)]
    pub notification_type: Option<String>,

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
