use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use super::{AgentStatus, Engine, SessionStatus, TmuxInfo};

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
    /// The reason this agent stopped for its next step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_status: Option<AgentStatus>,
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

#[cfg(test)]
mod tests {
    use super::*;

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
                session.pending_human_review_ids,
                session.agent_status,
            ),
            (None, None, false, BTreeSet::new(), None)
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
}
