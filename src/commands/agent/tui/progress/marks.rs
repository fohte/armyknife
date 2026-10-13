use crate::commands::agent::types::{DisplayStatus, Session, SessionStatus};
use crate::commands::agent::user_turn::{UserTurn, UserTurnKind};
use crate::infra::github::{PrCiState, PrProgressInfo, PrState};
use crate::shared::config::{AgentConfig, AgentStatusState};
use crate::shared::human_in_the_loop::PR_DRAFT_REVIEW_MARKER;

use super::{ProgressMark, StageMarks, user_turn_for};

pub(in crate::commands::agent::tui) fn session_mark(
    session: &Session,
    turn: Option<&UserTurn<'_>>,
    agent_config: &AgentConfig,
) -> ProgressMark {
    if session.status == SessionStatus::Ended {
        return ProgressMark::Done;
    }
    if turn.is_some() {
        return ProgressMark::User;
    }
    if session.status != SessionStatus::Running
        && let Some(status) = session.agent_status.as_ref()
    {
        match agent_config.status_state(&status.kind) {
            AgentStatusState::Done => return ProgressMark::Done,
            AgentStatusState::Waiting => return ProgressMark::Waiting,
            AgentStatusState::User => return ProgressMark::User,
        }
    }
    match session.display_status() {
        DisplayStatus::Running => ProgressMark::Working,
        DisplayStatus::Background => ProgressMark::Waiting,
        DisplayStatus::Failed | DisplayStatus::WaitingInput => ProgressMark::User,
        DisplayStatus::Ended => ProgressMark::Done,
        DisplayStatus::UnreadStopped | DisplayStatus::Stopped | DisplayStatus::Paused => {
            ProgressMark::NotStarted
        }
    }
}

pub(in crate::commands::agent::tui) fn stage_marks(
    session: &Session,
    progress: Option<&PrProgressInfo>,
    agent_config: &AgentConfig,
) -> StageMarks {
    let is_merged = progress.is_some_and(|progress| progress.pull_request.state == PrState::Merged);
    if is_merged {
        return completed_stage_marks();
    }

    let mut stages = session_stage_marks(session, progress.is_some(), agent_config);
    (stages.ci, stages.merge) = pr_ci_merge_marks(progress);
    stages
}

pub(in crate::commands::agent::tui) fn pr_ci_merge_marks(
    progress: Option<&PrProgressInfo>,
) -> (ProgressMark, ProgressMark) {
    match progress {
        None => (ProgressMark::NotStarted, ProgressMark::NotStarted),
        Some(progress) => {
            let ci = match progress.ci_state {
                PrCiState::Waiting => ProgressMark::Waiting,
                PrCiState::InProgress => ProgressMark::Working,
                PrCiState::Successful => ProgressMark::Done,
                PrCiState::Unknown => ProgressMark::NotStarted,
            };
            let merge = match progress.pull_request.state {
                PrState::Merged => ProgressMark::Done,
                PrState::Open if progress.ci_state == PrCiState::Successful => ProgressMark::User,
                PrState::Open
                    if matches!(
                        progress.ci_state,
                        PrCiState::Waiting | PrCiState::InProgress
                    ) =>
                {
                    ProgressMark::Waiting
                }
                PrState::Open | PrState::Closed => ProgressMark::NotStarted,
            };
            (ci, merge)
        }
    }
}

pub(in crate::commands::agent::tui) fn completed_stage_marks() -> StageMarks {
    StageMarks {
        implementation: ProgressMark::Done,
        diff_review: ProgressMark::Done,
        pr_body: ProgressMark::Done,
        ci: ProgressMark::Done,
        merge: ProgressMark::Done,
    }
}

pub(in crate::commands::agent::tui) fn session_stage_marks(
    session: &Session,
    has_pull_request: bool,
    agent_config: &AgentConfig,
) -> StageMarks {
    let turn = user_turn_for(session, agent_config);
    let current = session_mark(session, turn.as_ref(), agent_config);
    let has_crit = !session.crit_urls.is_empty();
    let has_pr_body_review = session
        .pending_human_review_ids
        .iter()
        .any(|id| id.starts_with(PR_DRAFT_REVIEW_MARKER));
    let implementation = if has_pull_request || has_crit || has_pr_body_review {
        ProgressMark::Done
    } else {
        current
    };
    let diff_review = if has_crit {
        active_review_mark(turn.as_ref(), UserTurnKind::Crit, current)
    } else if has_pr_body_review || has_pull_request {
        ProgressMark::Done
    } else {
        ProgressMark::NotStarted
    };
    let pr_body = if has_pr_body_review {
        active_review_mark(turn.as_ref(), UserTurnKind::Body, current)
    } else if has_pull_request {
        ProgressMark::Done
    } else {
        ProgressMark::NotStarted
    };

    StageMarks {
        implementation,
        diff_review,
        pr_body,
        ci: ProgressMark::NotStarted,
        merge: ProgressMark::NotStarted,
    }
}

pub(in crate::commands::agent::tui) fn active_review_mark(
    turn: Option<&UserTurn<'_>>,
    expected_kind: UserTurnKind,
    fallback: ProgressMark,
) -> ProgressMark {
    match turn {
        Some(turn) if turn.kind == expected_kind => ProgressMark::User,
        _ => fallback,
    }
}
