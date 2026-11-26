use std::collections::BTreeSet;
use std::path::PathBuf;

use chrono::{TimeZone, Utc};
use rstest::fixture;

use super::types::{Engine, Session, SessionStatus};

#[fixture]
pub(super) fn deferred_stop_delegate() -> Session {
    let timestamp = Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap();
    Session {
        launch_id: None,
        archived_at: None,
        pr_context: None,
        created_pr_urls: Vec::new(),
        session_id: "fixture-delegate".to_string(),
        crit_urls: Vec::new(),
        crit_not_waiting_urls: Default::default(),
        web_uis: Vec::new(),
        pending_human_review_ids: Default::default(),
        cwd: PathBuf::from("/fixture/worktree"),
        transcript_path: None,
        tty: None,
        tmux_info: None,
        status: SessionStatus::Stopped,
        created_at: timestamp,
        updated_at: timestamp,
        last_prompted_at: None,
        last_message: None,
        current_tool: None,
        current_tool_name: None,
        label: None,
        work_type: None,
        work_type_pinned: false,
        vibe_enabled: false,
        launched_with_vibe: false,
        child_launch: super::types::ChildLaunchPermission::Deny,
        ancestor_session_ids: vec!["fixture-parent".to_string()],
        pending_bg_task_ids: BTreeSet::new(),
        pending_agent_task_ids: BTreeSet::new(),
        pending_permission_agent_ids: BTreeSet::new(),
        pending_permission_request_ids: Default::default(),
        read_at: None,
        sweep_signaled: false,
        stop_blocked_by_armyknife: false,
        agent_status: None,
        agent_status_clear_pending: false,
        engine: Engine::Codex,
        model: None,
        effort: None,
    }
}
