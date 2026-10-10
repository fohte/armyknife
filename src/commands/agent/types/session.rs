use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use super::{DisplayStatus, Engine, SessionStatus, TmuxInfo};

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
    /// A process killed before cleanup can leave a marker; display status
    /// considers it only while some background task is still pending.
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
    /// waiting even while other agents keep firing events in parallel.
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

impl Session {
    /// A `Stopped` session is unread when it has never been focused since its
    /// most recent transition into `Stopped`. Drives the `✱` glyph.
    pub fn is_unread_stopped(&self) -> bool {
        self.status == SessionStatus::Stopped && self.read_at.is_none()
    }

    /// True if this session has a pending background task or Task-tool
    /// subagent. `pending_bg_task_ids` includes Claude Code task IDs and
    /// `a agent bg run`'s runtime marker. Shared by every consumer that must
    /// treat such a session as still mid-task despite an idle main loop:
    /// `auto_pause` (skip pausing), `auto_compact` (skip compacting), and
    /// `display_status` (report `Background` instead of `Stopped`, or
    /// `WaitingInput` for a stopped session with a linked crit review or
    /// pending Human-in-the-Loop review).
    pub fn has_pending_bg_tasks(&self) -> bool {
        !self.pending_bg_task_ids.is_empty() || !self.pending_agent_task_ids.is_empty()
    }

    /// True if any agent (main thread or subagent) in this session is
    /// currently blocked on a permission prompt. See
    /// `pending_permission_agent_ids` for what forces this to clear.
    pub fn has_pending_permission_requests(&self) -> bool {
        !self.pending_permission_agent_ids.is_empty()
    }

    /// Presentation status for this session. A stopped main loop with pending
    /// background work is `WaitingInput` when a crit review is linked or a
    /// Human-in-the-Loop review is pending, and `Background` otherwise.
    /// Notifications, `auto_pause`, `auto_compact`, and `sweep` read persisted
    /// status or `has_pending_bg_tasks` directly and must not switch to this.
    pub fn display_status(&self) -> DisplayStatus {
        if self.status == SessionStatus::Stopped && self.has_pending_bg_tasks() {
            return if !self.crit_urls.is_empty() || !self.pending_human_review_ids.is_empty() {
                DisplayStatus::WaitingInput
            } else {
                DisplayStatus::Background
            };
        }
        if self.is_unread_stopped() {
            return DisplayStatus::UnreadStopped;
        }
        match self.status {
            SessionStatus::Running => DisplayStatus::Running,
            SessionStatus::WaitingInput => DisplayStatus::WaitingInput,
            SessionStatus::Stopped => DisplayStatus::Stopped,
            SessionStatus::Paused => DisplayStatus::Paused,
            SessionStatus::Ended => DisplayStatus::Ended,
        }
    }

    /// Status symbol that also reflects unread state and in-flight
    /// background tasks. See `display_status`.
    pub fn display_symbol(&self) -> &'static str {
        self.display_status().display_symbol()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    fn session(status: SessionStatus, read_at: Option<DateTime<Utc>>) -> Session {
        Session {
            session_id: "s".to_string(),
            work_type: None,
            work_type_pinned: false,
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
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
            pending_permission_agent_ids: BTreeSet::new(),
            pending_permission_request_ids: Default::default(),
            read_at,
            sweep_signaled: false,
            engine: Engine::Claude,
        }
    }

    #[rstest]
    #[case::running_unread(SessionStatus::Running, None, false, false, "\u{25cf}")]
    #[case::running_read(SessionStatus::Running, Some(()), false, false, "\u{25cf}")]
    // A session actually running is `Running` regardless of a bg task -- only
    // an idle main loop with a pending bg task and no crit link renders as
    // `Background`.
    #[case::running_with_bg_task(SessionStatus::Running, None, true, false, "\u{25cf}")]
    #[case::stopped_with_bg_task(SessionStatus::Stopped, None, true, false, "\u{25ce}")]
    #[case::crit_review_waiting(SessionStatus::Stopped, Some(()), true, true, "\u{25d0}")]
    #[case::running_with_crit_review(SessionStatus::Running, None, true, true, "\u{25cf}")]
    #[case::crit_without_pending_task(SessionStatus::Stopped, Some(()), false, true, "\u{25cb}")]
    #[case::waiting_unread(SessionStatus::WaitingInput, None, false, false, "\u{25d0}")]
    #[case::stopped_unread(SessionStatus::Stopped, None, false, false, "\u{2731}")]
    #[case::stopped_read(SessionStatus::Stopped, Some(()), false, false, "\u{25cb}")]
    #[case::paused_unread(SessionStatus::Paused, None, false, false, "\u{23f8}")]
    #[case::paused_read(SessionStatus::Paused, Some(()), false, false, "\u{23f8}")]
    #[case::ended_unread(SessionStatus::Ended, None, false, false, "\u{25cb}")]
    #[case::ended_read(SessionStatus::Ended, Some(()), false, false, "\u{25cb}")]
    fn session_display_symbol_table(
        #[case] status: SessionStatus,
        #[case] read_marker: Option<()>,
        #[case] has_bg_task: bool,
        #[case] has_crit_link: bool,
        #[case] expected: &str,
    ) {
        let read_at = read_marker.map(|()| Utc::now());
        let mut s = session(status, read_at);
        if has_bg_task {
            s.pending_bg_task_ids.insert("bg-1".to_string());
        }
        if has_crit_link {
            s.crit_urls
                .push("https://crit.example/review/1".to_string());
        }
        assert_eq!(s.display_symbol(), expected);
    }

    #[rstest]
    #[case::stopped_with_pending_bg_task(SessionStatus::Stopped, true, DisplayStatus::WaitingInput)]
    #[case::stopped_without_pending_bg_task(SessionStatus::Stopped, false, DisplayStatus::Stopped)]
    #[case::running_with_pending_bg_task(SessionStatus::Running, true, DisplayStatus::Running)]
    fn human_review_wait_only_changes_pending_background_status(
        #[case] status: SessionStatus,
        #[case] has_pending_bg_task: bool,
        #[case] expected: DisplayStatus,
    ) {
        let mut session = session(status, Some(Utc::now()));
        if has_pending_bg_task {
            session.pending_bg_task_ids.insert("task-1".to_string());
        }
        session
            .pending_human_review_ids
            .insert("review-id".to_string());

        assert_eq!(session.display_status(), expected);
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
    #[case::empty(&[], false)]
    #[case::non_empty(&["agent-1"], true)]
    fn session_has_pending_permission_requests_table(
        #[case] pending: &[&str],
        #[case] expected: bool,
    ) {
        let mut s = session(SessionStatus::WaitingInput, None);
        s.pending_permission_agent_ids = pending.iter().map(|id| (*id).to_string()).collect();
        assert_eq!(s.has_pending_permission_requests(), expected);
    }
}
