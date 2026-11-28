use std::collections::BTreeSet;
use std::path::PathBuf;

use chrono::{TimeZone, Utc};
use rstest::fixture;

use super::types::{Engine, Session, SessionStatus};

pub(super) type AgentCardState = (
    Vec<(i64, Option<super::store::TodoResolution>, Option<String>)>,
    Vec<(
        i64,
        Option<super::store::ProposalResolution>,
        bool,
        Option<String>,
    )>,
);

pub(super) fn agent_card_state(
    sessions_dir: &std::path::Path,
    session_id: &str,
) -> anyhow::Result<AgentCardState> {
    let todos = super::store::agent_todos_from(sessions_dir, session_id)?;
    let proposals = super::store::agent_proposals_from(sessions_dir, session_id)?;
    Ok((
        todos
            .into_iter()
            .map(|todo| (todo.seq, todo.resolution, todo.comment))
            .collect(),
        proposals
            .into_iter()
            .map(|proposal| {
                (
                    proposal.seq,
                    proposal.resolution,
                    proposal.resolved_at.is_some(),
                    proposal.comment,
                )
            })
            .collect(),
    ))
}

pub(super) struct SessionEndCardFixture;

#[fixture]
pub(super) fn session_end_cards() -> SessionEndCardFixture {
    SessionEndCardFixture
}

impl SessionEndCardFixture {
    pub(super) fn add_to(&self, sessions_dir: &std::path::Path, session_id: &str) {
        let completed_todo = super::store::add_agent_todo_in(
            sessions_dir,
            session_id,
            "A finished task",
            None,
            false,
        )
        .expect("completed todo should be recorded");
        super::store::resolve_agent_todo_from(
            sessions_dir,
            session_id,
            completed_todo,
            super::store::TodoResolution::Completed,
            None,
        )
        .expect("todo should be completed");
        super::store::add_agent_todo_in(sessions_dir, session_id, "A pending task", None, false)
            .expect("pending todo should be recorded");

        let answered_proposal = super::store::add_agent_proposal_in(
            sessions_dir,
            session_id,
            "{\"title\":\"An answered choice\"}",
        )
        .expect("answered proposal should be recorded");
        super::store::write_agent_proposal_answer_at_from(
            sessions_dir,
            session_id,
            answered_proposal,
            "Selected an option.",
            Utc::now(),
        )
        .expect("proposal should be answered");
        super::store::add_agent_proposal_in(
            sessions_dir,
            session_id,
            "{\"title\":\"A pending choice\"}",
        )
        .expect("pending proposal should be recorded");
    }
}

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
