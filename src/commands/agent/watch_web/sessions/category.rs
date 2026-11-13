use chrono::{DateTime, Utc};

use crate::commands::agent::store;
use crate::commands::agent::types::{Session, SessionStatus};
use crate::commands::agent::user_turn::{UserTurnKind, user_turns};
use crate::shared::config::{AgentConfig, AgentStatusState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SessionCategory {
    NeedsAction,
    Later,
    InProgress,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "PascalCase")]
pub(super) enum SessionKind {
    Question,
    Approve,
    Review,
    Error,
    Plan,
    Stopped,
    Proposal,
    Todo,
}

pub(super) fn decision_category_is_done(
    session: &Session,
    agent_config: &AgentConfig,
    has_running_background_task: bool,
    state: Option<&store::WatchWebSessionState>,
    now: DateTime<Utc>,
) -> bool {
    let turn = user_turns(std::iter::once(session), agent_config)
        .into_iter()
        .next()
        .map(|turn| turn.kind);
    let agent_status_state = session
        .agent_status
        .as_ref()
        .map(|status| agent_config.status_state(&status.kind));
    category_for_with_later(
        session,
        turn,
        false,
        has_running_background_task,
        agent_status_state,
        state,
        now,
    ) == SessionCategory::Done
}

pub(super) fn category_for_with_later(
    session: &Session,
    turn: Option<UserTurnKind>,
    interrupted: bool,
    has_running_background_task: bool,
    agent_status_state: Option<AgentStatusState>,
    state: Option<&store::WatchWebSessionState>,
    now: DateTime<Utc>,
) -> SessionCategory {
    let still_unchanged = state.is_some_and(|state| {
        state
            .activity_snapshot
            .is_none_or(|snapshot| session.updated_at <= snapshot)
    });
    let later_is_active = still_unchanged
        && state.is_some_and(|state| {
            state.until_activity || state.later_until.is_some_and(|deadline| now < deadline)
        });
    if later_is_active {
        SessionCategory::Later
    } else {
        category_for_with_interruption(
            session,
            turn,
            interrupted,
            has_running_background_task,
            agent_status_state,
        )
    }
}

pub(super) fn category_for_with_interruption(
    session: &Session,
    turn: Option<UserTurnKind>,
    interrupted: bool,
    has_running_background_task: bool,
    agent_status_state: Option<AgentStatusState>,
) -> SessionCategory {
    if interrupted {
        SessionCategory::NeedsAction
    } else {
        category_for(
            session,
            turn,
            has_running_background_task,
            agent_status_state,
        )
    }
}

pub(super) fn category_for(
    session: &Session,
    turn: Option<UserTurnKind>,
    has_running_background_task: bool,
    agent_status_state: Option<AgentStatusState>,
) -> SessionCategory {
    if (turn.is_some()
        && !(session.status == SessionStatus::Stopped && turn == Some(UserTurnKind::AgentStatus)))
        || session.is_current_tool_named("ExitPlanMode")
    {
        SessionCategory::NeedsAction
    } else if session.status == SessionStatus::Stopped {
        match agent_status_state {
            Some(AgentStatusState::User) => SessionCategory::NeedsAction,
            Some(AgentStatusState::Waiting) => SessionCategory::InProgress,
            Some(AgentStatusState::Done) if !has_running_background_task => SessionCategory::Done,
            Some(AgentStatusState::Done) | None if has_running_background_task => {
                SessionCategory::InProgress
            }
            Some(AgentStatusState::Done) | None => SessionCategory::NeedsAction,
        }
    } else if session.status == SessionStatus::Running {
        SessionCategory::InProgress
    } else {
        SessionCategory::Done
    }
}

#[cfg(test)]
fn kind_for(
    session: &Session,
    turn: Option<UserTurnKind>,
    has_unanswered_proposal: bool,
    has_running_background_task: bool,
) -> Option<SessionKind> {
    kind_for_with_todo(
        session,
        turn,
        has_unanswered_proposal,
        false,
        has_running_background_task,
    )
}

pub(super) fn kind_for_with_todo(
    session: &Session,
    turn: Option<UserTurnKind>,
    has_unanswered_proposal: bool,
    has_unanswered_todo: bool,
    has_running_background_task: bool,
) -> Option<SessionKind> {
    if session.is_current_tool_named("ExitPlanMode") {
        return Some(SessionKind::Plan);
    }

    if turn == Some(UserTurnKind::Ask) {
        return Some(SessionKind::Question);
    }
    if turn == Some(UserTurnKind::Allow) {
        return Some(SessionKind::Approve);
    }
    if has_unanswered_proposal {
        return Some(SessionKind::Proposal);
    }
    if has_unanswered_todo {
        return Some(SessionKind::Todo);
    }

    let Some(turn) = turn else {
        return (session.status == SessionStatus::Stopped && !has_running_background_task)
            .then_some(SessionKind::Stopped);
    };

    match turn {
        UserTurnKind::Ask => Some(SessionKind::Question),
        UserTurnKind::Allow => Some(SessionKind::Approve),
        UserTurnKind::Crit | UserTurnKind::Body => Some(SessionKind::Review),
        UserTurnKind::Failed => Some(SessionKind::Error),
        UserTurnKind::AgentStatus => {
            let status = session.agent_status.as_ref()?;
            let content = format!("{} {}", status.kind, status.note).to_ascii_lowercase();
            let has_word = |cues: &[&str]| {
                content
                    .split(|character: char| !character.is_ascii_alphanumeric())
                    .any(|word| cues.contains(&word))
            };
            if content.contains('?') || has_word(&["question", "ask", "asked", "answer", "reply"]) {
                Some(SessionKind::Question)
            } else if has_word(&[
                "review",
                "reviews",
                "reviewing",
                "crit",
                "body",
                "approve",
                "approval",
                "proposal",
            ]) {
                Some(SessionKind::Review)
            } else {
                (!has_running_background_task).then_some(SessionKind::Stopped)
            }
        }
        UserTurnKind::Merge => Some(SessionKind::Review),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::types::{AgentStatus, Engine, TmuxInfo};
    use crate::shared::config::AgentStatusConfig;
    use chrono::TimeZone;
    use rstest::rstest;
    use std::path::PathBuf;

    fn session(status: SessionStatus) -> Session {
        Session {
            session_id: "session".to_string(),
            archived_at: None,
            pr_context: None,
            crit_urls: Vec::new(),
            web_uis: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: PathBuf::from("/workspace/project"),
            transcript_path: None,
            tty: None,
            tmux_info: None::<TmuxInfo>,
            status,
            agent_status: None,
            agent_status_clear_pending: false,
            created_at: Utc.timestamp_opt(1_000, 0).unwrap(),
            updated_at: Utc.timestamp_opt(1_000, 0).unwrap(),
            last_message: None,
            current_tool: None,
            current_tool_name: None,
            label: None,
            work_type: None,
            vibe_enabled: false,
            launched_with_vibe: false,
            work_type_pinned: false,
            child_launch: Default::default(),
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: Default::default(),
            pending_agent_task_ids: Default::default(),
            pending_permission_agent_ids: Default::default(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            engine: Engine::Claude,
            model: None,
            effort: None,
        }
    }

    #[rstest]
    #[case::waiting_tool(UserTurnKind::Ask, Some(SessionKind::Question))]
    #[case::permission(UserTurnKind::Allow, Some(SessionKind::Approve))]
    #[case::crit_review(UserTurnKind::Crit, Some(SessionKind::Review))]
    #[case::draft_review(UserTurnKind::Body, Some(SessionKind::Review))]
    #[case::failure(UserTurnKind::Failed, Some(SessionKind::Error))]
    #[case::merge(UserTurnKind::Merge, Some(SessionKind::Review))]
    fn maps_user_turn_kinds_to_web_kinds(
        #[case] turn: UserTurnKind,
        #[case] expected: Option<SessionKind>,
    ) {
        assert_eq!(
            kind_for(
                &session(SessionStatus::WaitingInput),
                Some(turn),
                false,
                false,
            ),
            expected
        );
    }

    #[rstest]
    #[case::agent_question("ask_user", "", false, Some(SessionKind::Question))]
    #[case::note_question("needs_attention", "Which option?", false, Some(SessionKind::Question))]
    #[case::agent_review("needs_review", "", false, Some(SessionKind::Review))]
    #[case::task_in_review_note(
        "needs_review",
        "Review the task plan",
        false,
        Some(SessionKind::Review)
    )]
    #[case::agent_stopped("waiting_for_external", "", false, Some(SessionKind::Stopped))]
    #[case::active_background_task("waiting_for_external", "", true, None)]
    fn maps_agent_status_to_web_kind(
        #[case] status_kind: &str,
        #[case] note: &str,
        #[case] has_running_background_task: bool,
        #[case] expected: Option<SessionKind>,
    ) {
        let mut session = session(SessionStatus::Stopped);
        session.agent_status = Some(AgentStatus {
            kind: status_kind.to_string(),
            note: note.to_string(),
        });

        assert_eq!(
            kind_for(
                &session,
                Some(UserTurnKind::AgentStatus),
                false,
                has_running_background_task,
            ),
            expected
        );
    }

    #[test]
    fn maps_exit_plan_mode_to_plan_kind() {
        let mut session = session(SessionStatus::WaitingInput);
        session.current_tool = Some("ExitPlanMode(fixture details)".to_string());
        session.current_tool_name = Some("ExitPlanMode".to_string());

        assert_eq!(
            kind_for(&session, Some(UserTurnKind::Ask), true, false),
            Some(SessionKind::Plan)
        );
    }

    #[test]
    fn legacy_exit_plan_mode_display_name_keeps_plan_classification() {
        let mut session = session(SessionStatus::WaitingInput);
        session.current_tool = Some("ExitPlanMode".to_string());

        assert_eq!(
            (
                kind_for(&session, Some(UserTurnKind::Ask), true, false),
                category_for(&session, None, false, None),
            ),
            (Some(SessionKind::Plan), SessionCategory::NeedsAction,),
        );
    }

    #[rstest]
    #[case::question(Some(UserTurnKind::Ask), SessionKind::Question)]
    #[case::approval(Some(UserTurnKind::Allow), SessionKind::Approve)]
    fn pending_question_and_approval_kinds_precede_proposals(
        #[case] turn: Option<UserTurnKind>,
        #[case] expected: SessionKind,
    ) {
        assert_eq!(
            kind_for(&session(SessionStatus::WaitingInput), turn, true, false),
            Some(expected),
        );
    }

    #[rstest]
    #[case::todo(None, SessionKind::Todo)]
    #[case::question(Some(UserTurnKind::Ask), SessionKind::Question)]
    #[case::approval(Some(UserTurnKind::Allow), SessionKind::Approve)]
    fn pending_todos_follow_questions_and_approvals(
        #[case] turn: Option<UserTurnKind>,
        #[case] expected: SessionKind,
    ) {
        assert_eq!(
            kind_for_with_todo(
                &session(SessionStatus::WaitingInput),
                turn,
                false,
                true,
                false,
            ),
            Some(expected),
        );
    }

    #[test]
    fn exit_plan_mode_needs_action() {
        let mut session = session(SessionStatus::WaitingInput);
        session.current_tool = Some("ExitPlanMode(fixture details)".to_string());
        session.current_tool_name = Some("ExitPlanMode".to_string());

        assert_eq!(
            category_for(&session, None, false, None),
            SessionCategory::NeedsAction
        );
    }

    #[test]
    fn stopped_without_a_user_turn_has_stopped_kind() {
        assert_eq!(
            kind_for(&session(SessionStatus::Stopped), None, false, false),
            Some(SessionKind::Stopped)
        );
    }

    #[rstest]
    #[case::stopped(SessionStatus::Stopped, None, SessionCategory::NeedsAction)]
    #[case::user_turn(
        SessionStatus::WaitingInput,
        Some(UserTurnKind::Ask),
        SessionCategory::NeedsAction
    )]
    #[case::running(SessionStatus::Running, None, SessionCategory::InProgress)]
    #[case::paused(SessionStatus::Paused, None, SessionCategory::Done)]
    #[case::ended(SessionStatus::Ended, None, SessionCategory::Done)]
    fn maps_sessions_to_inbox_categories(
        #[case] status: SessionStatus,
        #[case] turn: Option<UserTurnKind>,
        #[case] expected: SessionCategory,
    ) {
        assert_eq!(category_for(&session(status), turn, false, None), expected);
    }

    #[rstest]
    #[case::configured_user(
        "fixture_user_status",
        AgentStatusState::User,
        Some(UserTurnKind::AgentStatus),
        false,
        SessionCategory::NeedsAction,
        Some(SessionKind::Stopped)
    )]
    #[case::configured_user_with_background_task(
        "fixture_user_status",
        AgentStatusState::User,
        Some(UserTurnKind::AgentStatus),
        true,
        SessionCategory::NeedsAction,
        None
    )]
    #[case::configured_waiting(
        "fixture_waiting_status",
        AgentStatusState::Waiting,
        None,
        false,
        SessionCategory::InProgress,
        Some(SessionKind::Stopped)
    )]
    #[case::configured_done(
        "fixture_done_status",
        AgentStatusState::Done,
        None,
        false,
        SessionCategory::Done,
        Some(SessionKind::Stopped)
    )]
    fn stopped_agent_statuses_follow_configured_state(
        #[case] status_kind: &str,
        #[case] state: AgentStatusState,
        #[case] expected_turn: Option<UserTurnKind>,
        #[case] has_running_background_task: bool,
        #[case] expected_category: SessionCategory,
        #[case] expected_kind: Option<SessionKind>,
    ) {
        let mut session = session(SessionStatus::Stopped);
        session.agent_status = Some(AgentStatus {
            kind: status_kind.to_string(),
            note: "Waiting for a task result.".to_string(),
        });
        let mut agent_config = AgentConfig::default();
        agent_config
            .statuses
            .insert(status_kind.to_string(), AgentStatusConfig { state });
        let turn = user_turns([&session], &agent_config)
            .first()
            .map(|turn| turn.kind);

        assert_eq!(
            (
                turn,
                category_for(&session, turn, has_running_background_task, Some(state)),
                kind_for(&session, turn, false, has_running_background_task),
            ),
            (expected_turn, expected_category, expected_kind),
        );
    }

    #[rstest]
    #[case::done(AgentStatusState::Done, false, true)]
    #[case::waiting(AgentStatusState::Waiting, false, false)]
    #[case::user(AgentStatusState::User, false, false)]
    #[case::done_with_background_task(AgentStatusState::Done, true, false)]
    fn decision_routing_uses_stopped_status_and_background_task_state(
        #[case] state: AgentStatusState,
        #[case] has_running_background_task: bool,
        #[case] expected_is_done: bool,
    ) {
        let status_kind = "fixture_status";
        let mut session = session(SessionStatus::Stopped);
        session.agent_status = Some(AgentStatus {
            kind: status_kind.to_string(),
            note: "Waiting for external work.".to_string(),
        });
        let mut agent_config = AgentConfig::default();
        agent_config
            .statuses
            .insert(status_kind.to_string(), AgentStatusConfig { state });

        assert_eq!(
            decision_category_is_done(
                &session,
                &agent_config,
                has_running_background_task,
                None,
                Utc::now(),
            ),
            expected_is_done,
        );
    }
}
