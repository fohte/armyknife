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
}

pub(super) fn decision_category_is_done(
    session: &Session,
    agent_config: &AgentConfig,
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
        session.has_pending_bg_tasks(),
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
        || session
            .current_tool
            .as_deref()
            .is_some_and(|tool| tool.starts_with("ExitPlanMode"))
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

pub(super) fn kind_for(
    session: &Session,
    turn: Option<UserTurnKind>,
    has_unanswered_proposal: bool,
    has_running_background_task: bool,
) -> Option<SessionKind> {
    if session
        .current_tool
        .as_deref()
        .is_some_and(|tool| tool.starts_with("ExitPlanMode"))
    {
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
                Some(SessionKind::Stopped)
            }
        }
        UserTurnKind::Merge => Some(SessionKind::Review),
    }
}
