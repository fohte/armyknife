use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

use super::error::CcError;

/// Tmux user option name for storing Claude Code session ID.
/// User options in tmux are prefixed with '@' and persist until explicitly unset.
/// Uses a descriptive name to avoid conflicts with other potential armyknife options.
pub const TMUX_SESSION_OPTION: &str = "@armyknife-last-claude-code-session-id";

/// Tmux window-scoped user option holding the aggregated Claude Code status
/// symbols for the window. `a cc hook` writes it whenever a session's state
/// changes, so tmux's `window-status-format` can read `#{@armyknife-cc-window-status}`
/// directly instead of re-running `a cc window-status` on every redraw.
pub const TMUX_WINDOW_STATUS_OPTION: &str = "@armyknife-cc-window-status";

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
    /// IDs of background tasks (`Bash` with `run_in_background: true`) that
    /// were launched in this session and whose completion has not yet been
    /// observed. The Stop hook fires synthetically as soon as a bg task is
    /// spawned, so a non-empty set means "the user is still mid-task even
    /// though Claude's main loop went idle". Consumed by `auto_compact`
    /// (skip compaction while non-empty) and by `sweep` (do not auto-pause
    /// while non-empty). Cleared per-id when a `BashOutput` PostToolUse
    /// reports completion or a `KillShell` PostToolUse fires.
    #[serde(default)]
    pub pending_bg_task_ids: BTreeSet<String>,
    /// Output file paths for in-flight Task-tool subagents launched in this
    /// session (`Task` with `run_in_background: true`) whose completion has
    /// not yet been observed. Same rationale as `pending_bg_task_ids` (the
    /// Stop hook fires synthetically right after launch), but there is no
    /// completion hook to clear these eagerly -- Claude Code documents no
    /// confirmed hook firing for a background-launched subagent's completion
    /// -- so entries are only removed lazily by `sweep`'s lsof-based liveness
    /// probe (see `agent_task.rs`) once Claude Code closes the file. Consumed
    /// by `auto_compact` and `sweep` exactly like `pending_bg_task_ids`.
    #[serde(default)]
    pub pending_agent_task_outputs: BTreeSet<PathBuf>,
    /// Timestamp the user last focused this session via `a cc focus`.
    /// `None` means the session has never been focused since its last
    /// transition to `Stopped` (i.e. unread); `Some(_)` means read.
    /// Reset to `None` every time the session re-enters `Stopped` so a new
    /// idle turn re-surfaces as unread. Only meaningful while
    /// `status == Stopped`; other statuses ignore it.
    #[serde(default)]
    pub read_at: Option<DateTime<Utc>>,
    /// Set by `sweep::signal_session` when it (re-)sends SIGTERM without yet
    /// confirming the session as `Paused` (a live `claude` pid still
    /// resolved at signal time), so `status` stays `Stopped` in the
    /// meantime. While set, a `SessionEnd` hook firing on the still-Stopped
    /// session means the just-signaled process is exiting as a result of
    /// that signal, not that the user ended it themselves -- see the
    /// `SessionEnd` handler in `hook.rs`. Cleared by any other hook event
    /// (the process is still responding, so sweep's earlier signal is no
    /// longer relevant) and by `sweep::confirm_paused`.
    #[serde(default)]
    pub sweep_signaled: bool,
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
    /// Stopped session that was automatically terminated (SIGTERM) after the
    /// `auto_pause` timeout elapsed. The session file is preserved so that
    /// `cc resume` / `claude --resume` can restore the conversation.
    Paused,
    /// Session has ended (Ctrl+D / /exit). Kept on disk so that `claude -c`
    /// resume can restore label and ancestor chain. Garbage-collected after
    /// a retention period by `cleanup_stale_sessions`.
    Ended,
}

/// Semantic color of a session status, independent of the output medium.
///
/// Each renderer maps this to its own medium (ANSI escapes for the terminal
/// table, tmux style markup for the status bar), so the status-to-color
/// decision lives in one place and cannot drift between renderers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusColor {
    Green,
    Yellow,
    Gray,
    Dim,
}

impl Session {
    /// A `Stopped` session is unread when it has never been focused since its
    /// most recent transition into `Stopped`. Drives the `✱` glyph.
    pub fn is_unread_stopped(&self) -> bool {
        self.status == SessionStatus::Stopped && self.read_at.is_none()
    }

    /// Status symbol that also reflects unread state.
    pub fn display_symbol(&self) -> &'static str {
        if self.is_unread_stopped() {
            "✱"
        } else {
            self.status.display_symbol()
        }
    }
}

impl SessionStatus {
    pub fn display_symbol(&self) -> &'static str {
        match self {
            Self::Running => "●",
            Self::WaitingInput => "◐",
            Self::Stopped | Self::Ended => "○",
            Self::Paused => "⏸",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::WaitingInput => "waiting",
            Self::Stopped => "stopped",
            Self::Paused => "paused",
            Self::Ended => "ended",
        }
    }

    pub fn color(&self) -> StatusColor {
        match self {
            Self::Running => StatusColor::Green,
            Self::WaitingInput => StatusColor::Yellow,
            Self::Paused => StatusColor::Dim,
            Self::Stopped | Self::Ended => StatusColor::Gray,
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

    // Pre-tool-use / Post-tool-use / PermissionRequest event fields
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_input: Option<ToolInput>,

    /// PostToolUse only. Claude Code does not document a stable schema for
    /// this field — its shape varies per tool (object for Bash/Read/Write,
    /// array of content blocks for MCP tools, etc.) and Anthropic has
    /// declined to publish one (anthropics/claude-code#3671). Accept any
    /// JSON value and extract only what we need at the call site.
    #[serde(default)]
    pub tool_response: Option<serde_json::Value>,

    // Ignore other fields from Claude Code hooks
    #[serde(flatten)]
    _extra: serde_json::Value,
}

impl HookInput {
    /// Background task id set by Claude Code when the Bash tool was launched
    /// with `run_in_background: true`. The background task itself fires no
    /// completion hook, so this is the only signal armyknife has that the
    /// next Stop is "I kicked off a background command", not the end of a
    /// real turn. Returns `None` for any tool_response shape that does not
    /// carry the field (MCP arrays, Read/Write objects, missing field, etc.).
    pub fn background_task_id(&self) -> Option<&str> {
        self.tool_response
            .as_ref()?
            .get("backgroundTaskId")?
            .as_str()
    }

    /// Shell id referenced by `BashOutput` / `KillShell` tool calls. Claude
    /// Code's tool_input schema for these tools is not documented
    /// (anthropics/claude-code#3671); empirically the field is `shell_id`
    /// but `bash_id` has appeared in older builds. Try both.
    pub fn shell_id(&self) -> Option<&str> {
        let ti = self.tool_input.as_ref()?;
        ti.shell_id.as_deref().or(ti.bash_id.as_deref())
    }

    /// Status string from a `BashOutput` PostToolUse `tool_response`. Returns
    /// `None` for any shape that does not carry a string `status` field
    /// (still running, schema mismatch, MCP array payload, etc.).
    pub fn bash_output_status(&self) -> Option<&str> {
        self.tool_response.as_ref()?.get("status")?.as_str()
    }

    /// Output file path for a `Task` tool subagent launched with
    /// `run_in_background: true`. Present in `tool_response` alongside
    /// `"status": "async_launched"` (see
    /// https://code.claude.com/docs/en/hooks.md, PreToolUse input > Agent).
    /// Like the background bg task itself, the subagent fires no completion
    /// hook, so this is the only signal armyknife has that the immediately
    /// following Stop is synthetic rather than the end of a real turn.
    pub fn agent_task_output_file(&self) -> Option<&str> {
        let response = self.tool_response.as_ref()?;
        if response.get("status")?.as_str()? != "async_launched" {
            return None;
        }
        response.get("outputFile")?.as_str()
    }
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
    /// Shell id for `BashOutput` / `KillShell` tools (current Claude Code).
    #[serde(default)]
    pub shell_id: Option<String>,
    /// Shell id for `BashOutput` / `KillShell` tools (legacy field name).
    #[serde(default)]
    pub bash_id: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::path::PathBuf;

    fn session(status: SessionStatus, read_at: Option<DateTime<Utc>>) -> Session {
        Session {
            session_id: "s".to_string(),
            cwd: PathBuf::from("/tmp/test"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: BTreeSet::new(),
            pending_agent_task_outputs: BTreeSet::new(),
            read_at,
            sweep_signaled: false,
        }
    }

    #[rstest]
    #[case::running_unread(SessionStatus::Running, None, "\u{25cf}")]
    #[case::running_read(SessionStatus::Running, Some(()), "\u{25cf}")]
    #[case::waiting_unread(SessionStatus::WaitingInput, None, "\u{25d0}")]
    #[case::stopped_unread(SessionStatus::Stopped, None, "\u{2731}")]
    #[case::stopped_read(SessionStatus::Stopped, Some(()), "\u{25cb}")]
    #[case::paused_unread(SessionStatus::Paused, None, "\u{23f8}")]
    #[case::paused_read(SessionStatus::Paused, Some(()), "\u{23f8}")]
    #[case::ended_unread(SessionStatus::Ended, None, "\u{25cb}")]
    #[case::ended_read(SessionStatus::Ended, Some(()), "\u{25cb}")]
    fn session_display_symbol_table(
        #[case] status: SessionStatus,
        #[case] read_marker: Option<()>,
        #[case] expected: &str,
    ) {
        let read_at = read_marker.map(|()| Utc::now());
        assert_eq!(session(status, read_at).display_symbol(), expected);
    }

    #[test]
    fn read_at_defaults_to_none_when_missing_from_json() {
        // Existing on-disk sessions predate `read_at`; deserialization must
        // treat them as unread rather than failing.
        let json = serde_json::json!({
            "session_id": "legacy",
            "cwd": "/tmp/legacy",
            "transcript_path": null,
            "tmux_info": null,
            "status": "stopped",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_message": null,
        });
        let session: Session =
            serde_json::from_value(json).expect("legacy session should deserialize");
        assert_eq!(session.read_at, None);
    }
}
