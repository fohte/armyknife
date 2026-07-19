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
    /// IDs of in-flight Bash background tasks (`run_in_background: true`)
    /// launched in this session, as reported by Claude Code's own task
    /// registry (`background_tasks` on `Stop` input, filtered to
    /// `type == "shell"`; see `HookInput::pending_bg_task_ids`). The Stop
    /// hook fires synthetically as soon as a bg task is spawned, so a
    /// non-empty set means "the user is still mid-task even though Claude's
    /// main loop went idle". Overwritten wholesale from that array on every
    /// `Stop` event. Older Claude Code builds omit `background_tasks`
    /// entirely, which deserializes to an empty array and safely falls back
    /// to "nothing pending". `sweep` also clears this set early -- without
    /// waiting for a `Stop` -- once it can independently confirm no `claude`
    /// process resolves for the session (see `sweep/mod.rs`), so a crashed
    /// or killed process can't leave it stuck non-empty forever. Consumed by
    /// `auto_compact` (skip compaction while non-empty) and by `sweep` (do
    /// not auto-pause while non-empty).
    #[serde(default)]
    pub pending_bg_task_ids: BTreeSet<String>,
    /// IDs of in-flight Task-tool subagents launched in this session (`Task`
    /// with `run_in_background: true`), as reported by Claude Code's own task
    /// registry (`background_tasks` on `Stop` input, filtered to
    /// `type == "subagent"`; see `HookInput::pending_agent_task_ids`). Same
    /// rationale and refresh model as `pending_bg_task_ids` above, including
    /// `sweep`'s early clear once no `claude` process resolves. Consumed by
    /// `sweep` exactly like `pending_bg_task_ids`.
    ///
    /// `alias` accepts the field's pre-rename name so a session file
    /// written by an older `armyknife` build still deserializes instead of
    /// silently reverting to an empty set until the next `Stop`.
    #[serde(default, alias = "pending_agent_task_outputs")]
    pub pending_agent_task_ids: BTreeSet<String>,
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

    /// True if this session has a Bash background task or Task-tool subagent
    /// that has not yet reported completion (see `pending_bg_task_ids` /
    /// `pending_agent_task_ids`). Shared by every consumer that must treat
    /// such a session as still mid-task despite an idle main loop:
    /// `auto_pause` (skip pausing), `auto_compact` (skip compacting), and
    /// `determine_status` (skip the `Stop` -> `Stopped` transition).
    pub fn has_pending_bg_tasks(&self) -> bool {
        !self.pending_bg_task_ids.is_empty() || !self.pending_agent_task_ids.is_empty()
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

    /// Claude Code's own task registry snapshot. Per
    /// https://code.claude.com/docs/en/hooks.md (Stop input / SubagentStop
    /// input), Claude Code v2.1.145+ populates this on both `Stop` and
    /// `SubagentStop` input, but armyknife only wires the `Stop` hook (see
    /// `hook.rs`), so this is only ever read there. Older builds omit the
    /// field entirely, which deserializes to an empty vec. An entry
    /// disappears once its task is no longer in flight or scheduled -- the
    /// docs describe the array itself as empty whenever nothing is
    /// in-flight/scheduled, so presence in this list is the pending signal,
    /// not any particular `status` string.
    #[serde(default)]
    pub background_tasks: Vec<BackgroundTask>,

    // Ignore other fields from Claude Code hooks
    #[serde(flatten)]
    _extra: serde_json::Value,
}

impl HookInput {
    /// IDs of Bash background tasks (`run_in_background: true`) that Claude
    /// Code's task registry reports as still in flight or scheduled, per
    /// `background_tasks` (see its doc comment). Filtered to
    /// `type == "shell"` since `background_tasks` also covers Task-tool
    /// subagents (tracked separately via `pending_agent_task_ids`) and other
    /// task-registry entry types armyknife does not act on.
    pub fn pending_bg_task_ids(&self) -> BTreeSet<String> {
        self.pending_task_ids_of_type("shell")
    }

    /// IDs of Task-tool subagents that Claude Code's task registry reports
    /// as still in flight or scheduled, per `background_tasks` (see its doc
    /// comment). Filtered to `type == "subagent"` since `background_tasks`
    /// also covers Bash bg shells (tracked separately via
    /// `pending_bg_task_ids`) and other task-registry entry types armyknife
    /// does not act on.
    pub fn pending_agent_task_ids(&self) -> BTreeSet<String> {
        self.pending_task_ids_of_type("subagent")
    }

    /// True if `background_tasks` reports at least one Bash background task
    /// or Task-tool subagent still in flight or scheduled. Equivalent to
    /// `Session::has_pending_bg_tasks` once `Stop` has refreshed the
    /// session's pending sets from this same input; checking it directly
    /// here lets `determine_status` see the same signal before that refresh
    /// happens.
    pub fn has_pending_bg_tasks(&self) -> bool {
        self.background_tasks
            .iter()
            .any(|t| t.task_type == "shell" || t.task_type == "subagent")
    }

    fn pending_task_ids_of_type(&self, task_type: &str) -> BTreeSet<String> {
        self.background_tasks
            .iter()
            .filter(|t| t.task_type == task_type)
            .map(|t| t.id.clone())
            .collect()
    }
}

/// One entry of `HookInput::background_tasks`. Only `id` and `type` are
/// consumed by armyknife; `status`, `description`, `command`, `agent_type`
/// are accepted implicitly (serde ignores unlisted JSON keys).
#[derive(Debug, Deserialize)]
pub struct BackgroundTask {
    pub id: String,
    #[serde(rename = "type")]
    pub task_type: String,
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
            pending_agent_task_ids: BTreeSet::new(),
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

    #[test]
    fn pending_agent_task_ids_accepts_pre_rename_field_name() {
        // A session written by an older armyknife build (before
        // `pending_agent_task_outputs` was renamed to `pending_agent_task_ids`)
        // must still deserialize non-empty, not silently drop the pending
        // task and revert to an empty set until the next `Stop`.
        let json = serde_json::json!({
            "session_id": "legacy",
            "cwd": "/tmp/legacy",
            "transcript_path": null,
            "tmux_info": null,
            "status": "stopped",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_message": null,
            "pending_agent_task_outputs": ["/tmp/claude-1/proj/legacy/tasks/agent-1.output"],
        });
        let session: Session =
            serde_json::from_value(json).expect("legacy session should deserialize");
        assert_eq!(
            session.pending_agent_task_ids,
            BTreeSet::from(["/tmp/claude-1/proj/legacy/tasks/agent-1.output".to_string()])
        );
    }

    #[rstest]
    #[case::neither(false, false, false)]
    #[case::bg_only(true, false, true)]
    #[case::agent_only(false, true, true)]
    #[case::both(true, true, true)]
    fn session_has_pending_bg_tasks_table(
        #[case] bg_pending: bool,
        #[case] agent_pending: bool,
        #[case] expected: bool,
    ) {
        let mut s = session(SessionStatus::Stopped, None);
        if bg_pending {
            s.pending_bg_task_ids.insert("bg-1".to_string());
        }
        if agent_pending {
            s.pending_agent_task_ids.insert("agent-1".to_string());
        }
        assert_eq!(s.has_pending_bg_tasks(), expected);
    }

    #[rstest]
    #[case::empty("[]", false)]
    #[case::shell_task(r#"[{"id":"bg-1","type":"shell","status":"running"}]"#, true)]
    #[case::subagent_task(r#"[{"id":"task-1","type":"subagent","status":"running"}]"#, true)]
    #[case::other_type_only(r#"[{"id":"mon-1","type":"monitor","status":"running"}]"#, false)]
    fn hook_input_has_pending_bg_tasks_table(
        #[case] background_tasks_json: &str,
        #[case] expected: bool,
    ) {
        let json = format!(
            r#"{{"session_id":"s","cwd":"/tmp/test","background_tasks":{background_tasks_json}}}"#
        );
        let input: HookInput = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(input.has_pending_bg_tasks(), expected);
    }
}
