use std::collections::BTreeSet;
use std::path::PathBuf;

use chrono::Utc;
use rstest::fixture;

use super::super::types::{Engine, Session, SessionStatus, TmuxInfo};

#[fixture]
pub(super) fn session() -> Session {
    Session {
        archived_at: None,
        pr_context: None,
        session_id: "session-1".to_string(),
        work_type: None,
        work_type_pinned: false,
        vibe_enabled: false,
        launched_with_vibe: false,
        child_launch: Default::default(),
        crit_urls: Vec::new(),
        pending_human_review_ids: Default::default(),
        cwd: PathBuf::from("/tmp/test-repo"),
        transcript_path: None,
        tty: None,
        tmux_info: Some(TmuxInfo {
            session_name: "test".to_string(),
            window_name: "agent".to_string(),
            window_index: 1,
            pane_id: "%42".to_string(),
            socket_path: Some("/tmp/tmux-test/socket".to_string()),
        }),
        status: SessionStatus::Stopped,
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
        read_at: None,
        sweep_signaled: false,
        stop_blocked_by_armyknife: false,
        agent_status: None,
        agent_status_clear_pending: false,
        engine: Engine::Claude,
        model: None,
        effort: None,
    }
}
