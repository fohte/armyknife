use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::commands::agent::types::{Session, SessionStatus};
use crate::commands::agent::user_turn::{UserTurn, user_turns};
use crate::infra::github::PrProgressInfo;
use crate::shared::config::{AgentConfig, AgentStatusState};

use super::progress_batches::ImplementationPlan;

pub(super) mod marks;

use marks::{session_mark, session_stage_marks, stage_marks};

#[derive(Debug)]
pub(super) struct FetchedProgress {
    pub selected_session_id: String,
    pub request_scope: Option<ProgressFetchScope>,
    pub delegated: Vec<FetchedDelegate>,
    pub implementation: Option<ImplementationPlan>,
    pub github_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ProgressFetchScope {
    pub task_id: Option<String>,
    pub session_ids: Option<Vec<String>>,
}

#[derive(Debug)]
pub(super) struct FetchedDelegate {
    pub session: Session,
    pub checklist_item_id: Option<String>,
    pub pull_request: Option<PrProgressInfo>,
    pub lookup_unavailable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct StageMarks {
    pub implementation: ProgressMark,
    pub diff_review: ProgressMark,
    pub pr_body: ProgressMark,
    pub ci: ProgressMark,
    pub merge: ProgressMark,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum ProgressRow {
    Work(WorkProgressRow),
    PullRequest(PrProgressRow),
    ImplementationHeader {
        merged_pr_count: usize,
        total_pr_count: usize,
        collapsed: bool,
    },
    UnmatchedDelegatesHeader {
        collapsed: bool,
    },
    BatchHeader {
        batch_id: String,
        title: String,
        merged_pr_count: usize,
        collapsed: bool,
    },
    ChecklistPullRequest {
        batch_id: String,
        row: PrProgressRow,
    },
    PlannedPullRequest {
        batch_id: String,
        checklist_item_id: String,
        title: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ProgressSelectionKey {
    Session(String),
    Implementation,
    UnmatchedDelegates,
    Batch(String),
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
        ProgressRow::PullRequest(row) | ProgressRow::ChecklistPullRequest { row, .. } => (!row
            .is_ended)
            .then_some(row.pane_id.as_ref())
            .flatten()
            .map(|pane_id| ProgressOpenTarget::FocusPane(pane_id.clone())),
        ProgressRow::ImplementationHeader { .. }
        | ProgressRow::UnmatchedDelegatesHeader { .. }
        | ProgressRow::BatchHeader { .. }
        | ProgressRow::PlannedPullRequest { .. } => None,
    }
}

pub(super) fn open_pr_url_target(row: &ProgressRow) -> Option<ProgressOpenTarget> {
    match row {
        ProgressRow::PullRequest(row) | ProgressRow::ChecklistPullRequest { row, .. } => row
            .url
            .as_ref()
            .map(|url| ProgressOpenTarget::OpenUrl(url.clone())),
        ProgressRow::Work(_)
        | ProgressRow::ImplementationHeader { .. }
        | ProgressRow::UnmatchedDelegatesHeader { .. }
        | ProgressRow::BatchHeader { .. }
        | ProgressRow::PlannedPullRequest { .. } => None,
    }
}

impl ProgressRow {
    pub(super) fn session_id(&self) -> Option<&str> {
        match self {
            Self::Work(row) => Some(&row.session_id),
            Self::PullRequest(row) | Self::ChecklistPullRequest { row, .. } => {
                Some(&row.session_id)
            }
            Self::ImplementationHeader { .. }
            | Self::UnmatchedDelegatesHeader { .. }
            | Self::BatchHeader { .. }
            | Self::PlannedPullRequest { .. } => None,
        }
    }

    pub(super) fn selection_key(&self) -> Option<ProgressSelectionKey> {
        match self {
            Self::Work(row) => Some(ProgressSelectionKey::Session(row.session_id.clone())),
            Self::PullRequest(row) => Some(ProgressSelectionKey::Session(row.session_id.clone())),
            Self::ChecklistPullRequest { row, .. } => {
                Some(ProgressSelectionKey::Session(row.session_id.clone()))
            }
            Self::ImplementationHeader { .. } => Some(ProgressSelectionKey::Implementation),
            Self::UnmatchedDelegatesHeader { .. } => Some(ProgressSelectionKey::UnmatchedDelegates),
            Self::BatchHeader { batch_id, .. } => {
                Some(ProgressSelectionKey::Batch(batch_id.clone()))
            }
            Self::PlannedPullRequest { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct WorkProgressRow {
    pub session_id: String,
    pub role: String,
    pub mark: ProgressMark,
    pub status_label: Option<String>,
    pub note: Option<String>,
    #[serde(skip)]
    pub pane_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct PrProgressRow {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    pub title: String,
    pub number: Option<u64>,
    pub url: Option<String>,
    #[serde(skip)]
    pub pane_id: Option<String>,
    pub is_ended: bool,
    pub lookup_unavailable: bool,
    pub stages: StageMarks,
}

#[cfg(test)]
pub(super) fn build_progress_rows(
    selected: &Session,
    delegated: &[FetchedDelegate],
    agent_config: &AgentConfig,
) -> Vec<ProgressRow> {
    build_task_progress_rows(std::slice::from_ref(selected), delegated, agent_config)
}

pub(super) fn build_task_progress_rows(
    work_sessions: &[Session],
    delegated: &[FetchedDelegate],
    agent_config: &AgentConfig,
) -> Vec<ProgressRow> {
    let visible_delegates = delegated
        .iter()
        .filter_map(|delegate| {
            let row = pr_progress_row(delegate, agent_config);
            delegate_row_is_displayable(&row).then_some((delegate, row))
        })
        .collect::<Vec<_>>();
    let mut rows = Vec::with_capacity(work_sessions.len() + visible_delegates.len());
    rows.extend(work_sessions.iter().map(|session| {
        let has_delegated_work = has_delegated_descendant(
            session,
            visible_delegates
                .iter()
                .map(|(delegate, _)| &delegate.session),
        );
        ProgressRow::Work(work_progress_row(session, has_delegated_work, agent_config))
    }));
    rows.extend(
        visible_delegates
            .iter()
            .map(|(_, row)| ProgressRow::PullRequest(row.clone())),
    );
    rows
}

pub(super) fn delegate_row_is_displayable(row: &PrProgressRow) -> bool {
    !row.is_ended || row.number.is_some() || row.url.is_some()
}

pub(super) fn has_delegated_descendant<'a>(
    work_session: &Session,
    mut delegated: impl Iterator<Item = &'a Session>,
) -> bool {
    delegated.any(|session| {
        session
            .ancestor_session_ids
            .iter()
            .any(|ancestor_id| ancestor_id == &work_session.session_id)
    })
}

pub(super) fn work_row_count(rows: &[ProgressRow]) -> usize {
    rows.iter()
        .take_while(|row| matches!(row, ProgressRow::Work(_)))
        .count()
}

pub(super) fn table_selection_index(rows: &[ProgressRow], selected_index: usize) -> Option<usize> {
    selected_index.checked_sub(work_row_count(rows))
}

pub(super) fn work_progress_row(
    selected: &Session,
    has_delegated_work: bool,
    agent_config: &AgentConfig,
) -> WorkProgressRow {
    let turn = user_turn_for(selected, agent_config);
    let delegated_wait = has_delegated_work
        && selected.status != SessionStatus::Running
        && selected.agent_status.as_ref().is_some_and(|status| {
            agent_config.status_state(&status.kind) == AgentStatusState::Waiting
        });
    WorkProgressRow {
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
    }
}

pub(super) fn pr_progress_row(
    delegate: &FetchedDelegate,
    agent_config: &AgentConfig,
) -> PrProgressRow {
    let pull_request = delegate.pull_request.as_ref();
    let pr = pull_request.map(|progress| &progress.pull_request);
    let mut row = PrProgressRow {
        session_id: delegate.session.session_id.clone(),
        repo: delegate
            .session
            .pr_context
            .as_ref()
            .map(|context| context.repo.clone()),
        title: pr.map_or_else(|| session_title(&delegate.session), |pr| pr.title.clone()),
        number: pr.map(|pr| pr.number),
        url: pr.map(|pr| pr.url.clone()),
        pane_id: None,
        is_ended: false,
        lookup_unavailable: delegate.lookup_unavailable,
        stages: stage_marks(&delegate.session, pull_request, agent_config),
    };
    apply_session_fields(&mut row, &delegate.session);
    row
}

pub(super) fn repo_from_pr_url(url: &str) -> Option<&str> {
    lazy_regex::regex_captures!(r"^https?://[^/]+/(?:[^/]+/)+([^/]+)/pull/\d+(?:/.*)?$", url)
        .map(|(_, repo)| repo)
}

fn apply_session_fields(row: &mut PrProgressRow, session: &Session) {
    row.pane_id = session.tmux_info.as_ref().map(|info| info.pane_id.clone());
    row.is_ended = session.status == SessionStatus::Ended;
    if row.number.is_none() {
        row.title = session_title(session);
    }
}

#[cfg(test)]
pub(super) fn refresh_rows_from_sessions(
    selected: &Session,
    delegated: &[Session],
    existing_rows: &[ProgressRow],
    agent_config: &AgentConfig,
) -> Vec<ProgressRow> {
    refresh_task_rows_from_sessions(
        std::slice::from_ref(selected),
        &selected.session_id,
        delegated,
        existing_rows,
        agent_config,
    )
}

pub(super) fn refresh_task_rows_from_sessions(
    work_sessions: &[Session],
    selected_session_id: &str,
    delegated: &[Session],
    existing_rows: &[ProgressRow],
    agent_config: &AgentConfig,
) -> Vec<ProgressRow> {
    let has_existing_pr_rows = existing_rows.iter().any(|row| {
        matches!(
            row,
            ProgressRow::PullRequest(_) | ProgressRow::ChecklistPullRequest { .. }
        )
    });
    let mut rows = work_sessions
        .iter()
        .map(|work_session| {
            let has_delegated_work = has_delegated_descendant(work_session, delegated.iter())
                || (work_session.session_id == selected_session_id && has_existing_pr_rows);
            ProgressRow::Work(work_progress_row(
                work_session,
                has_delegated_work,
                agent_config,
            ))
        })
        .collect::<Vec<_>>();
    let mut previous = existing_rows
        .iter()
        .filter_map(|row| match row {
            ProgressRow::PullRequest(row) => Some((row.session_id.clone(), row.clone())),
            ProgressRow::ChecklistPullRequest { row, .. } => {
                Some((row.session_id.clone(), row.clone()))
            }
            ProgressRow::Work(_)
            | ProgressRow::ImplementationHeader { .. }
            | ProgressRow::UnmatchedDelegatesHeader { .. }
            | ProgressRow::BatchHeader { .. }
            | ProgressRow::PlannedPullRequest { .. } => None,
        })
        .collect::<HashMap<_, _>>();

    for session in delegated {
        let mut row = previous.remove(&session.session_id).unwrap_or_else(|| {
            pr_progress_row(
                &FetchedDelegate {
                    session: session.clone(),
                    checklist_item_id: None,
                    pull_request: None,
                    lookup_unavailable: false,
                },
                agent_config,
            )
        });
        apply_session_fields(&mut row, session);
        if row.stages.merge != ProgressMark::Done {
            let updated_stages = session_stage_marks(session, row.number.is_some(), agent_config);
            row.stages.implementation = updated_stages.implementation;
            row.stages.diff_review = updated_stages.diff_review;
            row.stages.pr_body = updated_stages.pr_body;
        }
        if delegate_row_is_displayable(&row) {
            rows.push(ProgressRow::PullRequest(row));
        }
    }

    // Ended delegates leave the live session list before their PR is resolved.
    for row in existing_rows {
        if let ProgressRow::PullRequest(row) = row
            && delegate_row_is_displayable(row)
            && !delegated
                .iter()
                .any(|session| session.session_id == row.session_id)
        {
            rows.push(ProgressRow::PullRequest(row.clone()));
        }
    }

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

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;

    use chrono::{TimeZone, Utc};
    use rstest::{fixture, rstest};

    use super::*;
    use crate::commands::agent::types::{AgentStatus, Engine};
    use crate::infra::github::{PrCiState, PrState};
    use crate::shared::config::{AgentStatusConfig, AgentStatusState};
    use crate::shared::human_in_the_loop::PR_DRAFT_REVIEW_MARKER;

    #[fixture]
    fn selected_session() -> Session {
        session("selected")
    }

    #[rstest]
    #[case::github_url(
        "https://github.com/example-owner/example-repo/pull/42",
        Some("example-repo")
    )]
    #[case::github_files_url(
        "https://github.com/example-owner/example-repo/pull/42/files",
        Some("example-repo")
    )]
    #[case::repository_named_pull(
        "https://github.com/example-owner/pull/example-repo/pull/42",
        Some("example-repo")
    )]
    #[case::url_without_repository("https://example.test/pull/42", None)]
    fn extracts_repository_from_pull_request_url(
        #[case] url: &str,
        #[case] expected: Option<&str>,
    ) {
        assert_eq!(repo_from_pr_url(url), expected);
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
            pr_context: None,
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
            launched_with_vibe: false,
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
                    checklist_item_id: None,
                    pull_request: None,
                    lookup_unavailable: false,
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
                    repo: None,
                    session_id: "delegate".to_string(),
                    title: "Example delegated work".to_string(),
                    number: None,
                    url: None,
                    pane_id: None,
                    is_ended: false,
                    lookup_unavailable: false,
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

    #[test]
    fn failed_session_uses_user_attention_mark_in_progress() {
        let mut selected = session("selected");
        selected.status = SessionStatus::Failed;

        assert_eq!(
            build_progress_rows(&selected, &[], &AgentConfig::default()),
            vec![ProgressRow::Work(WorkProgressRow {
                session_id: "selected".to_string(),
                role: "session".to_string(),
                mark: ProgressMark::User,
                status_label: Some("failed".to_string()),
                note: None,
                pane_id: None,
            })],
        );
    }

    #[test]
    fn unavailable_worktree_lookup_is_kept_on_the_delegate_row() {
        let selected = session("selected");

        assert_eq!(
            build_progress_rows(
                &selected,
                &[FetchedDelegate {
                    session: delegated_session(),
                    checklist_item_id: None,
                    pull_request: None,
                    lookup_unavailable: true,
                }],
                &AgentConfig::default(),
            ),
            vec![
                ProgressRow::Work(WorkProgressRow {
                    session_id: "selected".to_string(),
                    role: "session".to_string(),
                    mark: ProgressMark::Working,
                    status_label: None,
                    note: None,
                    pane_id: None,
                }),
                ProgressRow::PullRequest(PrProgressRow {
                    repo: None,
                    session_id: "delegate".to_string(),
                    title: "Example delegated work".to_string(),
                    number: None,
                    url: None,
                    pane_id: None,
                    is_ended: false,
                    lookup_unavailable: true,
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

    #[test]
    fn session_updates_refresh_rows_without_losing_lookup_state() {
        let mut selected = session("selected");
        selected.work_type = Some("updated role".to_string());
        selected.status = SessionStatus::Ended;
        let mut delegate = delegated_session();
        delegate.label = Some("Updated delegated work".to_string());
        delegate.status = SessionStatus::Ended;
        let previous_rows = build_progress_rows(
            &session("selected"),
            &[FetchedDelegate {
                session: delegated_session(),
                checklist_item_id: None,
                pull_request: None,
                lookup_unavailable: true,
            }],
            &AgentConfig::default(),
        );

        assert_eq!(
            refresh_rows_from_sessions(
                &selected,
                &[delegate],
                &previous_rows,
                &AgentConfig::default(),
            ),
            vec![ProgressRow::Work(WorkProgressRow {
                session_id: "selected".to_string(),
                role: "updated role".to_string(),
                mark: ProgressMark::Done,
                status_label: None,
                note: None,
                pane_id: None,
            }),],
        );
    }

    #[test]
    fn refresh_task_rows_keeps_delegated_wait_on_selected_session_when_peer_is_first() {
        let mut selected = session("selected");
        selected.status = SessionStatus::Stopped;
        selected.agent_status = Some(AgentStatus {
            kind: "external_wait".to_string(),
            note: "waiting for delegated work".to_string(),
        });
        let peer = session("peer");
        let delegated = delegated_session();
        let mut agent_config = AgentConfig::default();
        agent_config.statuses.insert(
            "external_wait".to_string(),
            AgentStatusConfig {
                state: AgentStatusState::Waiting,
            },
        );
        let previous_rows = build_progress_rows(
            &selected,
            &[FetchedDelegate {
                session: delegated,
                checklist_item_id: None,
                pull_request: None,
                lookup_unavailable: false,
            }],
            &agent_config,
        );

        assert_eq!(
            refresh_task_rows_from_sessions(
                &[peer, selected],
                "selected",
                &[],
                &previous_rows,
                &agent_config,
            ),
            vec![
                ProgressRow::Work(WorkProgressRow {
                    session_id: "peer".to_string(),
                    role: "session".to_string(),
                    mark: ProgressMark::Working,
                    status_label: None,
                    note: None,
                    pane_id: None,
                }),
                ProgressRow::Work(WorkProgressRow {
                    session_id: "selected".to_string(),
                    role: "session".to_string(),
                    mark: ProgressMark::Waiting,
                    status_label: None,
                    note: None,
                    pane_id: None,
                }),
                ProgressRow::PullRequest(PrProgressRow {
                    repo: None,
                    session_id: "delegate".to_string(),
                    title: "Example delegated work".to_string(),
                    number: None,
                    url: None,
                    pane_id: None,
                    is_ended: false,
                    lookup_unavailable: false,
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

    #[test]
    fn session_updates_keep_merged_pr_stages_done() {
        let selected = session("selected");
        let mut delegate = delegated_session();
        delegate.crit_urls.push("review-url".to_string());
        let previous_rows = build_progress_rows(
            &selected,
            &[FetchedDelegate {
                session: delegate.clone(),
                checklist_item_id: None,
                pull_request: Some(progress_info(PrState::Merged, PrCiState::Successful)),
                lookup_unavailable: false,
            }],
            &AgentConfig::default(),
        );

        assert_eq!(
            refresh_rows_from_sessions(
                &selected,
                &[delegate],
                &previous_rows,
                &AgentConfig::default(),
            ),
            vec![
                ProgressRow::Work(WorkProgressRow {
                    session_id: "selected".to_string(),
                    role: "session".to_string(),
                    mark: ProgressMark::Working,
                    status_label: None,
                    note: None,
                    pane_id: None,
                }),
                ProgressRow::PullRequest(PrProgressRow {
                    session_id: "delegate".to_string(),
                    repo: None,
                    title: "Example delegated work".to_string(),
                    number: Some(7),
                    url: Some("https://github.com/example-org/example-project/pull/7".to_string(),),
                    pane_id: None,
                    is_ended: false,
                    lookup_unavailable: false,
                    stages: StageMarks {
                        implementation: ProgressMark::Done,
                        diff_review: ProgressMark::Done,
                        pr_body: ProgressMark::Done,
                        ci: ProgressMark::Done,
                        merge: ProgressMark::Done,
                    },
                }),
            ],
        );
    }

    #[rstest]
    #[case::diff_review(true, false, PrState::Open, PrCiState::Unknown, StageMarks {
        implementation: ProgressMark::Done,
        diff_review: ProgressMark::User,
        pr_body: ProgressMark::Done,
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
    #[case::open_without_ci(false, false, PrState::Open, PrCiState::Unknown, StageMarks {
        implementation: ProgressMark::Done,
        diff_review: ProgressMark::Done,
        pr_body: ProgressMark::Done,
        ci: ProgressMark::NotStarted,
        merge: ProgressMark::NotStarted,
    })]
    #[case::open_and_ready_to_merge(false, false, PrState::Open, PrCiState::Successful, StageMarks {
        implementation: ProgressMark::Done,
        diff_review: ProgressMark::Done,
        pr_body: ProgressMark::Done,
        ci: ProgressMark::Done,
        merge: ProgressMark::User,
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
                    checklist_item_id: None,
                    pull_request: Some(progress_info(pr_state, ci_state)),
                    lookup_unavailable: false,
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
                    repo: None,
                    title: "Example delegated work".to_string(),
                    number: Some(7),
                    url: Some("https://github.com/example-org/example-project/pull/7".to_string()),
                    pane_id: None,
                    is_ended: false,
                    lookup_unavailable: false,
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
