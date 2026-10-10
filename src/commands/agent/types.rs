use anyhow::Result;
use chrono::{DateTime, Utc};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

mod status;

use super::error::CcError;

/// Which coding agent CLI hosts a session. Determines the process name to
/// look for in a tmux pane, and which binary/subcommand `a agent new` /
/// `a agent resume` launch. Defaults to `Claude` (`#[serde(default)]` on
/// `Session::engine`) so on-disk session files predating this field, and
/// hook invocations that don't pass `--engine`, keep behaving exactly as
/// before Codex support existed.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    #[default]
    Claude,
    Codex,
}

impl Engine {
    /// Process name to look for in a tmux pane's process tree.
    pub fn process_name(&self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
}

/// Effort levels accepted by both `claude --effort` and Codex `turn/start`.
/// A closed set so a typo is rejected
/// at parse time instead of reaching the CLI, which would silently fall back to
/// its default effort.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
    #[serde(rename = "xhigh")]
    #[value(name = "xhigh")]
    XHigh,
    Max,
}

impl ReasoningEffort {
    /// The wire value accepted by both Claude and Codex.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
        }
    }
}

/// Tmux user option name for storing an agent session ID.
/// User options in tmux are prefixed with '@' and persist until explicitly unset.
/// Uses a descriptive name to avoid conflicts with other potential armyknife options.
pub const TMUX_SESSION_OPTION: &str = "@armyknife-last-agent-session-id";

/// Pre-rename name of [`TMUX_SESSION_OPTION`]. Panes that were already open
/// when the rename shipped still carry it, and nothing rewrites the option
/// for a session that is already running, so reads fall back to it.
pub const TMUX_SESSION_OPTION_LEGACY: &str = "@armyknife-last-claude-code-session-id";

/// Resolves an agent session-id pane option by trying [`TMUX_SESSION_OPTION`]
/// first and falling back to [`TMUX_SESSION_OPTION_LEGACY`], so callers built
/// on either a direct tmux lookup or an abstraction over one (e.g.
/// `peer::wake`'s `Host` trait) share the same fallback order instead of
/// each re-deriving it.
pub fn resolve_session_option(read: impl Fn(&str) -> Option<String>) -> Option<String> {
    read(TMUX_SESSION_OPTION).or_else(|| read(TMUX_SESSION_OPTION_LEGACY))
}

/// Tmux window-scoped user option holding the aggregated Claude Code status
/// symbols for the window. `a agent hook` writes it whenever a session's state
/// changes, so tmux's `window-status-format` can read `#{@armyknife-cc-window-status}`
/// directly instead of re-running `a agent window-status` on every redraw.
pub const TMUX_WINDOW_STATUS_OPTION: &str = "@armyknife-cc-window-status";

/// Tmux window-scoped user option mirroring the `label` of the window's
/// (first, in pane order) labeled Claude Code session. Kept in sync
/// whenever `sync_window_option` runs, so `window-status-format` can show
/// a user-set title (via `a agent watch`'s rename key) without opening the
/// TUI. Empty when no session in the window has a label; tmux's own
/// `window-status-format` is expected to fall back to `#W` in that case.
pub const TMUX_WINDOW_TITLE_OPTION: &str = "@armyknife-cc-window-title";

/// Tmux pane user option mirroring the latest crit review URL for a session.
pub const TMUX_CRIT_OPTION: &str = "@crit";

/// Key used in `Session::pending_permission_agent_ids` for hook events fired
/// on the main thread (i.e. `HookInput::agent_id` is absent). Claude Code
/// never emits a real `agent_id` equal to this value: per
/// https://code.claude.com/docs/en/hooks.md, `agent_id` is only ever set
/// for hooks that fire inside a subagent.
pub const MAIN_THREAD_AGENT_KEY: &str = "__main__";

/// Runtime-only marker used to include `a agent bg run` tasks in shared
/// pending-background-task behavior without persisting registry state.
pub(crate) const BG_RUN_PENDING_TASK_MARKER: &str = "__armyknife_bg_run_pending__";

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub session_id: String,
    /// Crit review URLs opened by this session, ordered by most recently requested.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub crit_urls: Vec<String>,
    /// IDs of Human-in-the-Loop reviews currently waiting for the user.
    /// A process killed before cleanup can leave a marker. Claude Code status
    /// considers it only while a background task is pending; Codex uses it to
    /// identify a review wait that its hooks do not report as a task.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub pending_human_review_ids: BTreeSet<String>,
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
    /// Workflow skill name that identifies this session's work type.
    #[serde(default)]
    pub work_type: Option<String>,
    /// When true, hook-detected skills cannot replace the `a agent new --kind` value.
    #[serde(default, skip_serializing_if = "is_false")]
    pub work_type_pinned: bool,
    /// Ancestor session IDs from root to immediate parent.
    /// Used to build tree view: if intermediate sessions are deleted,
    /// child sessions can still find their nearest living ancestor.
    #[serde(default)]
    pub ancestor_session_ids: Vec<String>,
    /// IDs of in-flight background tasks launched in this session. Claude
    /// Code reports Bash tasks through its `background_tasks` Stop input
    /// (filtered to `type == "shell"`; see `HookInput::pending_bg_task_ids`).
    /// `a agent bg run` contributes `BG_RUN_PENDING_TASK_MARKER` at runtime
    /// after checking its separate task registry; that marker is removed
    /// before the session is persisted. A non-empty set means the user is
    /// still mid-task even if the agent's main loop went idle. `sweep` also
    /// clears stale Claude task IDs after confirming their process exited.
    /// Consumed by `auto_compact`, `sweep`, notifications, and display status.
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
    /// Keys of hook events currently blocked on a permission prompt
    /// (`PermissionRequest`), one per concurrently running agent. A key is
    /// either a subagent's `agent_id` or `MAIN_THREAD_AGENT_KEY` for the
    /// main thread. Non-empty forces `status` to `WaitingInput` regardless
    /// of what any other agent's event reports, so one subagent stuck on a
    /// permission prompt keeps the whole session (and its notification)
    /// waiting even while other subagents keep firing events in parallel.
    /// See the insert/remove/reconcile logic in `hook.rs`.
    #[serde(default)]
    pub pending_permission_agent_ids: BTreeSet<String>,
    /// Opaque ID for the latest permission request from each agent. The
    /// delayed notification worker uses this to distinguish a newer request
    /// for the same agent from the request that spawned it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pending_permission_request_ids: BTreeMap<String, String>,
    /// Timestamp the user last focused this session via `a agent focus`.
    /// `None` means the session has never been focused since its last
    /// transition to `Stopped` (i.e. unread); `Some(_)` means read.
    /// Reset to `None` every time the session re-enters `Stopped` so a new
    /// idle turn re-surfaces as unread. Only meaningful while
    /// `status == Stopped`; other statuses ignore it.
    #[serde(default)]
    pub read_at: Option<DateTime<Utc>>,
    /// Set by `sweep::signal_session` when it requests shutdown without yet
    /// confirming the session as `Paused` (a live agent pid still resolved
    /// at request time), so `status` stays `Stopped` in the
    /// meantime. While set, a `SessionEnd` hook firing on the still-Stopped
    /// session means the process is exiting as a result of that request,
    /// not that the user ended it themselves -- see the
    /// `SessionEnd` handler in `hook.rs`. Cleared by any other hook event
    /// (the process is still responding, so sweep's earlier signal is no
    /// longer relevant) and by `sweep::confirm_paused`.
    #[serde(default)]
    pub sweep_signaled: bool,
    /// Which coding agent CLI this session belongs to. See `Engine`.
    #[serde(default)]
    pub engine: Engine,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Stopped session that was automatically terminated after the
    /// `auto_pause` timeout elapsed. The session file is preserved so that
    /// `agent resume` can restore the conversation.
    Paused,
    /// Session has ended. Claude Code confirms this through SessionEnd;
    /// Codex requires a confirmed CLI exit, close, or thread takeover. Kept
    /// on disk for resume metadata and garbage-collected after a retention
    /// period.
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
    Cyan,
    Gray,
    Dim,
}

/// Presentation-only status derived from session status, unread state,
/// pending background tasks, and review state. See
/// `Session::display_status`.
///
/// Deliberately not `Serialize`/`Deserialize`: this is a derived, presentation-only
/// view and must never leak into the on-disk `Session`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayStatus {
    Running,
    WaitingInput,
    Stopped,
    /// `Stopped` session that hasn't been focused since its most recent
    /// transition into `Stopped`. See `Session::is_unread_stopped`.
    UnreadStopped,
    Paused,
    Ended,
    /// The main loop is stopped with a pending background task and no linked
    /// crit or Human-in-the-Loop review. See `Session::has_pending_bg_tasks`.
    Background,
}

/// Common fields present in all hook events.
#[derive(Debug, Deserialize)]
pub struct HookInput {
    pub session_id: String,
    pub cwd: PathBuf,
    #[serde(default)]
    pub transcript_path: Option<PathBuf>,

    /// Which coding agent CLI fired this hook. Never present in the JSON
    /// payload itself (`#[serde(default)]` -- neither Claude Code nor Codex
    /// send an `engine` field); `hook.rs::run` overwrites this from
    /// `HookArgs::engine` (the `--engine` CLI flag) right after parsing.
    #[serde(default)]
    pub engine: Engine,

    // SessionStart event fields
    /// Source of the session start event: "startup" (new session) or "resume" (session restore).
    /// Used to skip "startup" events on `claude -c` which create unwanted empty sessions.
    #[serde(default)]
    pub source: Option<String>,

    // UserPromptSubmit event fields
    #[serde(default)]
    pub prompt: Option<String>,

    // Notification event fields
    #[serde(default)]
    pub notification_type: Option<String>,

    // Pre-tool-use / Post-tool-use / PermissionRequest event fields
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_input: Option<ToolInput>,

    /// Identifies which agent fired this hook event. Present only when the
    /// event fires inside a subagent (per
    /// https://code.claude.com/docs/en/hooks.md); absent for main-thread
    /// events. `Stop`'s `background_tasks[].id` uses the same value for a
    /// `type == "subagent"` entry, which is what lets the `Stop` handler in
    /// `hook.rs` reconcile `Session::pending_permission_agent_ids` against
    /// still-live subagents.
    #[serde(default)]
    pub agent_id: Option<String>,

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

    /// The `Stop` hook payload's final assistant message of the turn, which
    /// avoids re-reading the transcript (see `hook.rs`'s `last_message`
    /// update). Always present for Codex; absent for older Claude Code
    /// versions, which fall back to the transcript.
    #[serde(default)]
    pub last_assistant_message: Option<String>,

    // Ignore other fields from Claude Code / Codex hooks
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
#[derive(Debug, PartialEq, Eq)]
pub struct ToolInput {
    /// Command for Bash tool
    pub command: Option<String>,
    /// Skill name for Claude Code's Skill tool
    pub skill: Option<String>,
    /// File path for Read/Write/Edit tools
    pub file_path: Option<String>,
    /// Pattern for Grep/Glob tools
    pub pattern: Option<String>,
}

// Never fails: Codex passes a tool's raw arguments as `tool_input`, so it can
// be a bare string or hold non-string values under these keys.
impl<'de> Deserialize<'de> for ToolInput {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let field = |key: &str| value.get(key)?.as_str().map(str::to_string);
        Ok(Self {
            command: field("command"),
            skill: field("skill"),
            file_path: field("file_path"),
            pattern: field("pattern"),
        })
    }
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

    // The CLI flag, the YAML config, and the value handed to `codex` must all
    // spell each effort the same way.
    #[rstest]
    #[case::low(ReasoningEffort::Low, "low")]
    #[case::medium(ReasoningEffort::Medium, "medium")]
    #[case::high(ReasoningEffort::High, "high")]
    #[case::xhigh(ReasoningEffort::XHigh, "xhigh")]
    #[case::max(ReasoningEffort::Max, "max")]
    fn reasoning_effort_spelling_is_consistent(
        #[case] effort: ReasoningEffort,
        #[case] expected: &str,
    ) {
        assert_eq!(
            (
                effort.as_str(),
                serde_yaml::to_string(&effort).unwrap(),
                effort.to_possible_value().unwrap().get_name().to_string(),
            ),
            (expected, format!("{expected}\n"), expected.to_string(),),
        );
    }

    #[rstest]
    #[case::prefers_current_key(Some("current"), Some("legacy"), Some("current"))]
    #[case::falls_back_to_legacy_key(None, Some("legacy"), Some("legacy"))]
    #[case::none_when_neither_set(None, None, None)]
    fn resolve_session_option_cases(
        #[case] current: Option<&'static str>,
        #[case] legacy: Option<&'static str>,
        #[case] expected: Option<&'static str>,
    ) {
        let resolved = resolve_session_option(|option| match option {
            TMUX_SESSION_OPTION => current.map(str::to_string),
            TMUX_SESSION_OPTION_LEGACY => legacy.map(str::to_string),
            _ => None,
        });
        assert_eq!(resolved, expected.map(str::to_string));
    }

    #[test]
    fn optional_fields_default_when_missing_from_json() {
        // Existing on-disk sessions predate these fields and must still load.
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
        assert_eq!(
            (
                session.read_at,
                session.work_type,
                session.work_type_pinned,
                session.pending_human_review_ids
            ),
            (None, None, false, BTreeSet::new())
        );
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

    fn tool_input(
        command: Option<&str>,
        file_path: Option<&str>,
        pattern: Option<&str>,
    ) -> ToolInput {
        ToolInput {
            command: command.map(str::to_string),
            skill: None,
            file_path: file_path.map(str::to_string),
            pattern: pattern.map(str::to_string),
        }
    }

    // Codex forwards a tool's raw arguments as `tool_input`, so the hook must
    // survive shapes Claude Code never sends.
    #[rstest]
    #[case::bash(r#"{"command":"ls"}"#, tool_input(Some("ls"), None, None))]
    #[case::read(r#"{"file_path":"/a"}"#, tool_input(None, Some("/a"), None))]
    #[case::grep(r#"{"pattern":"x"}"#, tool_input(None, None, Some("x")))]
    #[case::bare_string(r#""raw arguments""#, tool_input(None, None, None))]
    #[case::array(r#"["a","b"]"#, tool_input(None, None, None))]
    #[case::empty_object("{}", tool_input(None, None, None))]
    #[case::non_string_values(
        r#"{"command":["a","b"],"file_path":1,"pattern":null}"#,
        tool_input(None, None, None)
    )]
    fn tool_input_deserializes_any_shape(#[case] json: &str, #[case] expected: ToolInput) {
        assert_eq!(serde_json::from_str::<ToolInput>(json).unwrap(), expected);
    }
}
