use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use super::{AgentStatus, Engine, SessionStatus, TmuxInfo};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum ChildLaunchPermission {
    #[default]
    Allow,
    Deny,
}

impl ChildLaunchPermission {
    pub(crate) fn as_env_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }

    pub(crate) fn from_env(value: Option<&str>) -> Self {
        match value {
            Some("allow") => Self::Allow,
            Some("deny") | Some(_) => Self::Deny,
            None => Self::Allow,
        }
    }

    fn is_default(&self) -> bool {
        matches!(self, Self::Allow)
    }
}

impl<'de> serde::Deserialize<'de> for ChildLaunchPermission {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "kebab-case")]
        enum Permission {
            Allow,
            Deny,
        }

        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Value {
            Legacy(bool),
            Permission(Permission),
        }

        match Value::deserialize(deserializer)? {
            Value::Legacy(true) | Value::Permission(Permission::Deny) => Ok(Self::Deny),
            Value::Legacy(false) | Value::Permission(Permission::Allow) => Ok(Self::Allow),
        }
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub session_id: String,
    /// When set, the session is hidden from active listings until restored or collected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
    /// GitHub lookup data retained after a worktree is removed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_context: Option<SessionPrContext>,
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
    /// Preserve `agent_status` across status-only events until activity resumes.
    #[serde(default, skip_serializing_if = "is_false")]
    pub agent_status_clear_pending: bool,
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
    /// When true, this session explicitly enabled vibe mode.
    #[serde(default, skip_serializing_if = "is_false")]
    pub vibe_enabled: bool,
    /// Whether this session inherits vibe mode from an enabled ancestor.
    /// Set by `a agent new` when the caller has vibe enabled, or by `a agent vibe grant`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub launched_with_vibe: bool,
    /// When true, hook-detected skills cannot replace the `a agent new --kind` value.
    #[serde(default, skip_serializing_if = "is_false")]
    pub work_type_pinned: bool,
    /// Whether this session may launch child sessions.
    #[serde(
        default,
        alias = "is_delegated",
        skip_serializing_if = "ChildLaunchPermission::is_default"
    )]
    pub child_launch: ChildLaunchPermission,
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
    /// `alias` accepts the field's pre-rename name so a stored session record
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
    /// Set when armyknife blocks a Stop for a missing status, so the next
    /// Stop can keep that blocked turn's final message.
    #[serde(default, skip_serializing_if = "is_false")]
    pub stop_blocked_by_armyknife: bool,
    /// Which coding agent CLI this session belongs to. See `Engine`.
    #[serde(default)]
    pub engine: Engine,
}

impl Session {
    pub(crate) fn restore_from_archive(&mut self) -> bool {
        self.archived_at.take().is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPrContext {
    pub owner: String,
    pub repo: String,
    pub branch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checklist_item_id: Option<String>,
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
                session.archived_at,
                session.read_at,
                session.work_type,
                session.vibe_enabled,
                session.launched_with_vibe,
                session.child_launch,
                session.work_type_pinned,
                session.pending_human_review_ids,
                session.agent_status,
                session.stop_blocked_by_armyknife,
                session.agent_status_clear_pending,
                session.pr_context,
            ),
            (
                None,
                None,
                None,
                false,
                false,
                ChildLaunchPermission::Allow,
                false,
                BTreeSet::new(),
                None,
                false,
                false,
                None
            )
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

    #[rstest::rstest]
    #[case::legacy_true(
        "is_delegated",
        serde_json::Value::Bool(true),
        ChildLaunchPermission::Deny
    )]
    #[case::legacy_false(
        "is_delegated",
        serde_json::Value::Bool(false),
        ChildLaunchPermission::Allow
    )]
    #[case::current_deny("child_launch", serde_json::Value::String("deny".to_string()), ChildLaunchPermission::Deny)]
    #[case::current_allow("child_launch", serde_json::Value::String("allow".to_string()), ChildLaunchPermission::Allow)]
    fn child_launch_permission_migrates_legacy_and_current_values(
        #[case] field: &str,
        #[case] field_value: serde_json::Value,
        #[case] expected: ChildLaunchPermission,
    ) {
        let mut value = serde_json::json!({
            "session_id": "legacy",
            "cwd": "/tmp/example",
            "transcript_path": null,
            "tmux_info": null,
            "status": "stopped",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_message": null,
        });
        value[field] = field_value;
        let session: Session =
            serde_json::from_value(value).expect("session permission should deserialize");
        let serialized =
            serde_json::to_value(&session).expect("session permission should serialize");
        let actual = serde_json::json!({
            "permission": session.child_launch.as_env_str(),
            "serialized_child_launch": serialized.get("child_launch"),
            "legacy_field_present": serialized.get("is_delegated").is_some(),
        });
        let expected_permission = match expected {
            ChildLaunchPermission::Allow => "allow",
            ChildLaunchPermission::Deny => "deny",
        };
        let expected_serialized_child_launch = match expected {
            ChildLaunchPermission::Allow => serde_json::Value::Null,
            ChildLaunchPermission::Deny => serde_json::json!("deny"),
        };
        assert_eq!(
            actual,
            serde_json::json!({
                "permission": expected_permission,
                "serialized_child_launch": expected_serialized_child_launch,
                "legacy_field_present": false,
            })
        );
    }

    #[rstest::rstest]
    #[case::unspecified_defaults_to_allow(None, ChildLaunchPermission::Allow)]
    #[case::explicit_allow(Some("allow"), ChildLaunchPermission::Allow)]
    #[case::explicit_deny(Some("deny"), ChildLaunchPermission::Deny)]
    #[case::invalid_value_fails_closed(Some("unknown"), ChildLaunchPermission::Deny)]
    fn child_launch_permission_reads_environment(
        #[case] value: Option<&str>,
        #[case] expected: ChildLaunchPermission,
    ) {
        assert_eq!(ChildLaunchPermission::from_env(value), expected);
    }
}
