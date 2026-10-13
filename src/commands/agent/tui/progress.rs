use crate::commands::agent::types::{DisplayStatus, Session, SessionStatus};
use crate::commands::agent::user_turn::{UserTurn, UserTurnKind, user_turns};
use crate::infra::github::{PrCiState, PrProgressInfo, PrState};
use crate::shared::config::{AgentConfig, AgentStatusState};
use crate::shared::human_in_the_loop::PR_DRAFT_REVIEW_MARKER;

#[derive(Debug)]
pub(super) struct FetchedProgress {
    pub selected_session_id: String,
    pub delegated: Vec<FetchedDelegate>,
    pub github_error: Option<String>,
}

#[derive(Debug)]
pub(super) struct FetchedDelegate {
    pub session: Session,
    pub pull_request: Option<PrProgressInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProgressMark {
    Done,
    User,
    Working,
    Waiting,
    NotStarted,
}

impl ProgressMark {
    pub(super) fn symbol(self) -> &'static str {
        match self {
            Self::Done => "✓",
            Self::User => "◐",
            Self::Working => "●",
            Self::Waiting => "◷",
            Self::NotStarted => "○",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct StageMarks {
    pub implementation: ProgressMark,
    pub diff_review: ProgressMark,
    pub pr_body: ProgressMark,
    pub ci: ProgressMark,
    pub merge: ProgressMark,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ProgressRow {
    Work(WorkProgressRow),
    PullRequest(PrProgressRow),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ProgressOpenTarget {
    FocusPane(String),
    OpenUrl(String),
}

pub(super) fn open_target(row: &ProgressRow) -> Option<ProgressOpenTarget> {
    match row {
        ProgressRow::Work(row) => row
            .pane_id
            .as_ref()
            .map(|pane_id| ProgressOpenTarget::FocusPane(pane_id.clone())),
        ProgressRow::PullRequest(row) => {
            if !row.is_ended
                && let Some(pane_id) = row.pane_id.as_ref()
            {
                return Some(ProgressOpenTarget::FocusPane(pane_id.clone()));
            }
            row.url
                .as_ref()
                .map(|url| ProgressOpenTarget::OpenUrl(url.clone()))
        }
    }
}

impl ProgressRow {
    pub(super) fn session_id(&self) -> &str {
        match self {
            Self::Work(row) => &row.session_id,
            Self::PullRequest(row) => &row.session_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkProgressRow {
    pub session_id: String,
    pub role: String,
    pub mark: ProgressMark,
    pub status_label: Option<String>,
    pub note: Option<String>,
    pub pane_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PrProgressRow {
    pub session_id: String,
    pub title: String,
    pub number: Option<u64>,
    pub url: Option<String>,
    pub pane_id: Option<String>,
    pub is_ended: bool,
    pub stages: StageMarks,
}

pub(super) fn build_progress_rows(
    selected: &Session,
    delegated: &[FetchedDelegate],
    agent_config: &AgentConfig,
) -> Vec<ProgressRow> {
    let has_delegated_work = !delegated.is_empty();
    let turn = user_turn_for(selected, agent_config);
    let delegated_wait = has_delegated_work
        && selected.status != SessionStatus::Running
        && selected.agent_status.as_ref().is_some_and(|status| {
            agent_config.status_state(&status.kind) == AgentStatusState::Waiting
        });
    let work = WorkProgressRow {
        session_id: selected.session_id.clone(),
        role: selected
            .work_type
            .as_deref()
            .filter(|kind| !kind.is_empty())
            .or(selected.label.as_deref())
            .unwrap_or("session")
            .to_string(),
        mark: session_mark(selected, turn.as_ref(), agent_config),
        status_label: if delegated_wait {
            None
        } else {
            turn.as_ref()
                .and_then(|turn| turn.label)
                .or_else(|| {
                    selected
                        .agent_status
                        .as_ref()
                        .map(|status| status.kind.as_str())
                })
                .map(str::to_owned)
        },
        note: if delegated_wait {
            None
        } else {
            turn.as_ref()
                .and_then(|turn| turn.note)
                .or_else(|| {
                    selected
                        .agent_status
                        .as_ref()
                        .map(|status| status.note.as_str())
                })
                .filter(|note| !note.is_empty())
                .map(str::to_owned)
        },
        pane_id: selected.tmux_info.as_ref().map(|info| info.pane_id.clone()),
    };

    let mut rows = Vec::with_capacity(delegated.len() + 1);
    rows.push(ProgressRow::Work(work));
    rows.extend(delegated.iter().map(|delegate| {
        let pull_request = delegate.pull_request.as_ref();
        let pr = pull_request.map(|progress| &progress.pull_request);
        ProgressRow::PullRequest(PrProgressRow {
            session_id: delegate.session.session_id.clone(),
            title: pr.map_or_else(|| session_title(&delegate.session), |pr| pr.title.clone()),
            number: pr.map(|pr| pr.number),
            url: pr.map(|pr| pr.url.clone()),
            pane_id: delegate
                .session
                .tmux_info
                .as_ref()
                .map(|info| info.pane_id.clone()),
            is_ended: delegate.session.status == SessionStatus::Ended,
            stages: stage_marks(&delegate.session, pull_request, agent_config),
        })
    }));

    rows
}

fn session_title(session: &Session) -> String {
    session
        .label
        .as_deref()
        .filter(|label| !label.is_empty())
        .or(session.work_type.as_deref())
        .unwrap_or(&session.session_id)
        .to_string()
}

fn user_turn_for<'a>(session: &'a Session, agent_config: &AgentConfig) -> Option<UserTurn<'a>> {
    user_turns(std::iter::once(session), agent_config)
        .into_iter()
        .next()
}

fn session_mark(
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
        DisplayStatus::WaitingInput => ProgressMark::User,
        DisplayStatus::Ended => ProgressMark::Done,
        DisplayStatus::UnreadStopped | DisplayStatus::Stopped | DisplayStatus::Paused => {
            ProgressMark::NotStarted
        }
    }
}

fn stage_marks(
    session: &Session,
    progress: Option<&PrProgressInfo>,
    agent_config: &AgentConfig,
) -> StageMarks {
    let turn = user_turn_for(session, agent_config);
    let current = session_mark(session, turn.as_ref(), agent_config);
    let is_merged = progress.is_some_and(|progress| progress.pull_request.state == PrState::Merged);
    if is_merged {
        return StageMarks {
            implementation: ProgressMark::Done,
            diff_review: ProgressMark::Done,
            pr_body: ProgressMark::Done,
            ci: ProgressMark::Done,
            merge: ProgressMark::Done,
        };
    }

    let has_crit = !session.crit_urls.is_empty();
    let has_pr_body_review = session
        .pending_human_review_ids
        .iter()
        .any(|id| id.starts_with(PR_DRAFT_REVIEW_MARKER));
    let ci_state = progress.map(|progress| progress.ci_state);
    let ci_observed = matches!(
        ci_state,
        Some(PrCiState::Waiting | PrCiState::InProgress | PrCiState::Successful)
    );
    let implementation = if progress.is_some() || has_crit || has_pr_body_review {
        ProgressMark::Done
    } else {
        current
    };
    let diff_review = if has_crit {
        active_review_mark(turn.as_ref(), UserTurnKind::Crit, current)
    } else if has_pr_body_review || ci_observed {
        ProgressMark::Done
    } else {
        ProgressMark::NotStarted
    };
    let pr_body = if has_pr_body_review {
        active_review_mark(turn.as_ref(), UserTurnKind::Body, current)
    } else if ci_observed {
        ProgressMark::Done
    } else {
        ProgressMark::NotStarted
    };

    let (ci, merge) = match progress {
        None => (ProgressMark::NotStarted, ProgressMark::NotStarted),
        Some(progress) => {
            let ci_mark = match progress.ci_state {
                PrCiState::Waiting => ProgressMark::Waiting,
                PrCiState::InProgress => ProgressMark::Working,
                PrCiState::Successful => ProgressMark::Done,
                PrCiState::Unknown => ProgressMark::NotStarted,
            };
            let merge_mark = match progress.pull_request.state {
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
            (ci_mark, merge_mark)
        }
    };

    StageMarks {
        implementation,
        diff_review,
        pr_body,
        ci,
        merge,
    }
}

fn active_review_mark(
    turn: Option<&UserTurn<'_>>,
    expected_kind: UserTurnKind,
    fallback: ProgressMark,
) -> ProgressMark {
    match turn {
        Some(turn) if turn.kind == expected_kind => ProgressMark::User,
        _ => fallback,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;

    use chrono::{TimeZone, Utc};
    use rstest::{fixture, rstest};

    use super::*;
    use crate::commands::agent::types::{AgentStatus, Engine};
    use crate::shared::config::{AgentStatusConfig, AgentStatusState};

    #[fixture]
    fn selected_session() -> Session {
        session("selected")
    }

    #[fixture]
    fn delegated_session() -> Session {
        let mut session = session("delegate");
        session.is_delegated = true;
        session.ancestor_session_ids = vec!["selected".to_string()];
        session.label = Some("Example delegated work".to_string());
        session
    }

    fn session(id: &str) -> Session {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        Session {
            session_id: id.to_string(),
            crit_urls: Vec::new(),
            pending_human_review_ids: BTreeSet::new(),
            cwd: PathBuf::from("/tmp/example-project/.worktrees/example-branch"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            agent_status: None,
            agent_status_clear_pending: false,
            created_at: now,
            updated_at: now,
            last_message: None,
            current_tool: None,
            label: None,
            work_type: None,
            vibe_enabled: false,
            work_type_pinned: false,
            is_delegated: false,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: BTreeSet::new(),
            pending_agent_task_ids: BTreeSet::new(),
            pending_permission_agent_ids: BTreeSet::new(),
            pending_permission_request_ids: BTreeMap::new(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            engine: Engine::Claude,
        }
    }

    fn progress_info(state: PrState, ci_state: PrCiState) -> PrProgressInfo {
        PrProgressInfo {
            pull_request: crate::infra::github::PrInfo {
                number: 7,
                title: "Example delegated work".to_string(),
                state,
                url: "https://github.com/example-org/example-project/pull/7".to_string(),
            },
            ci_state,
        }
    }

    #[test]
    fn delegated_wait_is_shown_by_the_child_row_only() {
        let mut selected = session("selected");
        selected.status = SessionStatus::Stopped;
        selected.work_type = Some("project-manager".to_string());
        selected.agent_status = Some(AgentStatus {
            kind: "external_wait".to_string(),
            note: "waiting for delegated work".to_string(),
        });
        let delegate = delegated_session();
        let mut agent_config = AgentConfig::default();
        agent_config.statuses.insert(
            "external_wait".to_string(),
            AgentStatusConfig {
                state: AgentStatusState::Waiting,
            },
        );

        assert_eq!(
            build_progress_rows(
                &selected,
                &[FetchedDelegate {
                    session: delegate,
                    pull_request: None,
                }],
                &agent_config,
            ),
            vec![
                ProgressRow::Work(WorkProgressRow {
                    session_id: "selected".to_string(),
                    role: "project-manager".to_string(),
                    mark: ProgressMark::Waiting,
                    status_label: None,
                    note: None,
                    pane_id: None,
                }),
                ProgressRow::PullRequest(PrProgressRow {
                    session_id: "delegate".to_string(),
                    title: "Example delegated work".to_string(),
                    number: None,
                    url: None,
                    pane_id: None,
                    is_ended: false,
                    stages: StageMarks {
                        implementation: ProgressMark::Working,
                        diff_review: ProgressMark::NotStarted,
                        pr_body: ProgressMark::NotStarted,
                        ci: ProgressMark::NotStarted,
                        merge: ProgressMark::NotStarted,
                    },
                }),
            ],
        );
    }

    #[rstest]
    #[case::diff_review(true, false, PrState::Open, PrCiState::Unknown, StageMarks {
        implementation: ProgressMark::Done,
        diff_review: ProgressMark::User,
        pr_body: ProgressMark::NotStarted,
        ci: ProgressMark::NotStarted,
        merge: ProgressMark::NotStarted,
    })]
    #[case::pr_body_review(false, true, PrState::Open, PrCiState::Unknown, StageMarks {
        implementation: ProgressMark::Done,
        diff_review: ProgressMark::Done,
        pr_body: ProgressMark::User,
        ci: ProgressMark::NotStarted,
        merge: ProgressMark::NotStarted,
    })]
    #[case::ci_running(false, false, PrState::Open, PrCiState::InProgress, StageMarks {
        implementation: ProgressMark::Done,
        diff_review: ProgressMark::Done,
        pr_body: ProgressMark::Done,
        ci: ProgressMark::Working,
        merge: ProgressMark::Waiting,
    })]
    #[case::merged(false, false, PrState::Merged, PrCiState::Successful, StageMarks {
        implementation: ProgressMark::Done,
        diff_review: ProgressMark::Done,
        pr_body: ProgressMark::Done,
        ci: ProgressMark::Done,
        merge: ProgressMark::Done,
    })]
    fn stages_follow_active_reviews_and_pr_status(
        mut selected_session: Session,
        mut delegated_session: Session,
        #[case] active_crit: bool,
        #[case] active_body_review: bool,
        #[case] pr_state: PrState,
        #[case] ci_state: PrCiState,
        #[case] expected_stages: StageMarks,
    ) {
        if active_crit || active_body_review {
            delegated_session.status = SessionStatus::WaitingInput;
        }
        if active_crit {
            delegated_session
                .crit_urls
                .push("https://example.com/review".to_string());
        }
        if active_body_review {
            delegated_session
                .pending_human_review_ids
                .insert(format!("{PR_DRAFT_REVIEW_MARKER}example-review"));
        }
        selected_session.work_type = Some("design".to_string());

        assert_eq!(
            build_progress_rows(
                &selected_session,
                &[FetchedDelegate {
                    session: delegated_session,
                    pull_request: Some(progress_info(pr_state, ci_state)),
                }],
                &AgentConfig::default(),
            ),
            vec![
                ProgressRow::Work(WorkProgressRow {
                    session_id: "selected".to_string(),
                    role: "design".to_string(),
                    mark: ProgressMark::Working,
                    status_label: None,
                    note: None,
                    pane_id: None,
                }),
                ProgressRow::PullRequest(PrProgressRow {
                    session_id: "delegate".to_string(),
                    title: "Example delegated work".to_string(),
                    number: Some(7),
                    url: Some("https://github.com/example-org/example-project/pull/7".to_string()),
                    pane_id: None,
                    is_ended: false,
                    stages: expected_stages,
                }),
            ],
        );
    }

    #[test]
    fn unknown_custom_user_status_uses_its_configured_name() {
        let mut selected = session("selected");
        selected.status = SessionStatus::Stopped;
        selected.agent_status = Some(AgentStatus {
            kind: "needs_review".to_string(),
            note: "review the example plan".to_string(),
        });
        let mut config = AgentConfig::default();
        config.statuses.insert(
            "needs_review".to_string(),
            AgentStatusConfig {
                state: AgentStatusState::User,
            },
        );

        assert_eq!(
            build_progress_rows(&selected, &[], &config),
            vec![ProgressRow::Work(WorkProgressRow {
                session_id: "selected".to_string(),
                role: "session".to_string(),
                mark: ProgressMark::User,
                status_label: Some("needs_review".to_string()),
                note: Some("review the example plan".to_string()),
                pane_id: None,
            })],
        );
    }
}
