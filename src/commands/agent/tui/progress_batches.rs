use std::collections::{HashMap, HashSet};

use crate::commands::agent::types::Session;
use crate::infra::github::{PrNumberQuery, PrProgressInfo, PrState};
use crate::infra::tq::{TqChecklist, TqChecklistItem, TqGithubLink};
use crate::shared::config::AgentConfig;

use super::progress::marks::{completed_stage_marks, pr_ci_merge_marks};
use super::progress::{
    FetchedDelegate, PrProgressRow, ProgressMark, ProgressRow, StageMarks,
    delegate_row_is_displayable, has_delegated_descendant, pr_progress_row, work_progress_row,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ImplementationPlan {
    batches: Vec<ImplementationBatch>,
    unmatched_delegates: Vec<UnmatchedDelegate>,
}

impl ImplementationPlan {
    pub(super) fn batch_id_for_delegate(&self, session_id: &str) -> Option<&str> {
        self.batches.iter().find_map(|batch| {
            batch
                .items
                .iter()
                .any(|item| item.delegate_session_id.as_deref() == Some(session_id))
                .then_some(batch.id.as_str())
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImplementationBatch {
    id: String,
    title: String,
    items: Vec<ImplementationItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImplementationItem {
    id: String,
    title: String,
    delegate_session_id: Option<String>,
    github_link: Option<TqGithubLink>,
    pull_request: Option<PrProgressInfo>,
    lookup_unavailable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UnmatchedDelegate {
    session_id: String,
    pull_request: Option<PrProgressInfo>,
    lookup_unavailable: bool,
}

impl ImplementationItem {
    fn is_merged(&self) -> bool {
        self.pull_request
            .as_ref()
            .map(|progress| progress.pull_request.state == PrState::Merged)
            .or_else(|| {
                self.github_link
                    .as_ref()
                    .and_then(|link| link.state.as_deref())
                    .map(|state| state.eq_ignore_ascii_case("merged"))
            })
            .unwrap_or(false)
    }

    fn has_pull_request(&self) -> bool {
        self.pull_request.is_some() || self.github_link.is_some()
    }
}

pub(super) fn pull_request_queries(
    checklist: &TqChecklist,
    github_links: &[TqGithubLink],
) -> Vec<PrNumberQuery> {
    let github_links = github_links
        .iter()
        .map(|link| (link.id.as_str(), link))
        .collect::<HashMap<_, _>>();
    let mut queries = Vec::new();
    let mut seen = HashSet::new();

    for batch in &checklist.items {
        let mut leaves = Vec::new();
        collect_pull_request_leaves(batch, &mut leaves);
        for item in leaves {
            let Some(link_id) = item.github_link_id.as_deref() else {
                continue;
            };
            let Some(link) = github_links.get(link_id) else {
                continue;
            };
            let Some(query) = pr_number_query(link) else {
                continue;
            };
            if seen.insert(query.key()) {
                queries.push(query);
            }
        }
    }

    queries
}

pub(super) fn build_implementation_plan(
    checklist: &TqChecklist,
    github_links: &[TqGithubLink],
    delegates: &[FetchedDelegate],
    pull_requests: &HashMap<(String, String, u64), Option<PrProgressInfo>>,
    github_lookup_failed: bool,
) -> ImplementationPlan {
    let github_links = github_links
        .iter()
        .map(|link| (link.id.as_str(), link))
        .collect::<HashMap<_, _>>();
    let mut batches = Vec::new();
    let mut matched_delegate_session_ids = HashSet::new();

    for batch in &checklist.items {
        let mut leaves = Vec::new();
        collect_pull_request_leaves(batch, &mut leaves);
        let mut items = Vec::new();

        for leaf in leaves {
            let Some(title) = pull_request_title(&leaf.content) else {
                continue;
            };
            let github_link = leaf
                .github_link_id
                .as_deref()
                .and_then(|id| github_links.get(id).copied())
                .filter(|link| is_pull_request_link(link))
                .cloned();
            let delegate = delegates
                .iter()
                .find(|delegate| delegate.checklist_item_id.as_deref() == Some(&leaf.id));
            if let Some(delegate) = delegate {
                matched_delegate_session_ids.insert(delegate.session.session_id.clone());
            }
            let linked_progress = github_link.as_ref().and_then(|link| {
                let key = pr_number_query(link)?.key();
                pull_requests.get(&key).and_then(Option::as_ref)
            });
            let pull_request = linked_progress
                .cloned()
                .or_else(|| delegate.and_then(|delegate| delegate.pull_request.clone()));
            let linked_query_missing = github_link.as_ref().is_some_and(|link| {
                pr_number_query(link).is_some_and(|query| {
                    github_lookup_failed
                        || pull_requests.get(&query.key()).is_some_and(Option::is_none)
                })
            });
            let lookup_unavailable = pull_request.is_none()
                && (linked_query_missing
                    || delegate.is_some_and(|delegate| delegate.lookup_unavailable));

            items.push(ImplementationItem {
                id: leaf.id.clone(),
                title,
                delegate_session_id: delegate.map(|delegate| delegate.session.session_id.clone()),
                github_link,
                pull_request,
                lookup_unavailable,
            });
        }

        if !items.is_empty() {
            batches.push(ImplementationBatch {
                id: batch.id.clone(),
                title: batch.content.clone(),
                items,
            });
        }
    }

    let unmatched_delegates = delegates
        .iter()
        .filter(|delegate| !matched_delegate_session_ids.contains(&delegate.session.session_id))
        .map(|delegate| UnmatchedDelegate {
            session_id: delegate.session.session_id.clone(),
            pull_request: delegate.pull_request.clone(),
            lookup_unavailable: delegate.lookup_unavailable,
        })
        .collect();

    ImplementationPlan {
        batches,
        unmatched_delegates,
    }
}

#[cfg(test)]
pub(super) fn build_progress_rows(
    selected: &Session,
    delegates: &[Session],
    plan: &ImplementationPlan,
    implementation_expanded: bool,
    expanded_batches: &HashMap<String, bool>,
    agent_config: &AgentConfig,
) -> Vec<ProgressRow> {
    build_task_progress_rows(
        std::slice::from_ref(selected),
        delegates,
        plan,
        implementation_expanded,
        true,
        expanded_batches,
        agent_config,
    )
}

pub(super) fn build_task_progress_rows(
    work_sessions: &[Session],
    delegates: &[Session],
    plan: &ImplementationPlan,
    implementation_expanded: bool,
    unmatched_expanded: bool,
    expanded_batches: &HashMap<String, bool>,
    agent_config: &AgentConfig,
) -> Vec<ProgressRow> {
    let mut rows = work_sessions
        .iter()
        .map(|work_session| {
            let has_delegated_work = has_delegated_descendant(work_session, delegates.iter());
            ProgressRow::Work(work_progress_row(
                work_session,
                has_delegated_work,
                agent_config,
            ))
        })
        .collect::<Vec<_>>();
    let merged_pr_count = plan
        .batches
        .iter()
        .flat_map(|batch| &batch.items)
        .filter(|item| item.is_merged())
        .count();
    let total_pr_count = plan.batches.iter().map(|batch| batch.items.len()).sum();
    rows.push(ProgressRow::ImplementationHeader {
        merged_pr_count,
        total_pr_count,
        collapsed: !implementation_expanded,
    });
    if !implementation_expanded {
        append_unmatched_delegate_rows(
            &mut rows,
            delegates,
            plan,
            unmatched_expanded,
            agent_config,
        );
        return rows;
    }

    for batch in &plan.batches {
        let merged_count = batch.items.iter().filter(|item| item.is_merged()).count();
        let collapsed_by_default = !batch.items.is_empty() && merged_count == batch.items.len();
        let expanded = expanded_batches
            .get(&batch.id)
            .copied()
            .unwrap_or(!collapsed_by_default);
        let collapsed = !expanded;
        rows.push(ProgressRow::BatchHeader {
            batch_id: batch.id.clone(),
            title: batch.title.clone(),
            merged_pr_count: merged_count,
            collapsed,
        });
        if collapsed {
            continue;
        }

        for item in &batch.items {
            let delegate = item
                .delegate_session_id
                .as_deref()
                .and_then(|id| delegates.iter().find(|session| session.session_id == id));
            if let Some(session) = delegate {
                let mut row = pr_progress_row(
                    &FetchedDelegate {
                        session: session.clone(),
                        checklist_item_id: Some(item.id.clone()),
                        pull_request: item.pull_request.clone(),
                        lookup_unavailable: item.lookup_unavailable,
                    },
                    agent_config,
                );
                row.title = item.title.clone();
                apply_link_fallback(&mut row, item.github_link.as_ref());
                if item.is_merged() {
                    row.stages = completed_stage_marks();
                }
                rows.push(ProgressRow::ChecklistPullRequest {
                    batch_id: batch.id.clone(),
                    row,
                });
            } else if item.has_pull_request() {
                rows.push(ProgressRow::ChecklistPullRequest {
                    batch_id: batch.id.clone(),
                    row: pr_only_row(item),
                });
            } else {
                rows.push(ProgressRow::PlannedPullRequest {
                    batch_id: batch.id.clone(),
                    checklist_item_id: item.id.clone(),
                    title: item.title.clone(),
                });
            }
        }
    }

    append_unmatched_delegate_rows(&mut rows, delegates, plan, unmatched_expanded, agent_config);
    rows
}

fn append_unmatched_delegate_rows(
    rows: &mut Vec<ProgressRow>,
    delegates: &[Session],
    plan: &ImplementationPlan,
    expanded: bool,
    agent_config: &AgentConfig,
) {
    let mapped_sessions = plan
        .batches
        .iter()
        .flat_map(|batch| &batch.items)
        .filter_map(|item| item.delegate_session_id.as_deref())
        .collect::<HashSet<_>>();
    let unmatched_rows = delegates
        .iter()
        .filter(|session| !mapped_sessions.contains(session.session_id.as_str()))
        .filter_map(|session| {
            let fetched = plan
                .unmatched_delegates
                .iter()
                .find(|delegate| delegate.session_id == session.session_id);
            let row = pr_progress_row(
                &FetchedDelegate {
                    session: session.clone(),
                    checklist_item_id: None,
                    pull_request: fetched.and_then(|delegate| delegate.pull_request.clone()),
                    lookup_unavailable: fetched.is_some_and(|delegate| delegate.lookup_unavailable),
                },
                agent_config,
            );
            delegate_row_is_displayable(&row).then_some(row)
        })
        .collect::<Vec<_>>();
    if unmatched_rows.is_empty() {
        return;
    }
    rows.push(ProgressRow::UnmatchedDelegatesHeader {
        collapsed: !expanded,
    });
    if expanded {
        rows.extend(unmatched_rows.into_iter().map(ProgressRow::PullRequest));
    }
}

fn pr_only_row(item: &ImplementationItem) -> PrProgressRow {
    let progress = item.pull_request.as_ref();
    let link = item.github_link.as_ref();
    let is_merged = item.is_merged();
    PrProgressRow {
        session_id: format!("checklist-item:{}", item.id),
        title: item.title.clone(),
        number: progress
            .map(|progress| progress.pull_request.number)
            .or_else(|| link.and_then(|link| link.number)),
        url: progress
            .map(|progress| progress.pull_request.url.clone())
            .or_else(|| link.and_then(|link| link.url.clone())),
        pane_id: None,
        is_ended: true,
        lookup_unavailable: item.lookup_unavailable,
        stages: if is_merged {
            completed_stage_marks()
        } else {
            pr_only_stage_marks(progress)
        },
    }
}

fn pr_only_stage_marks(progress: Option<&PrProgressInfo>) -> StageMarks {
    let (ci, merge) = pr_ci_merge_marks(progress);
    StageMarks {
        implementation: ProgressMark::Done,
        diff_review: ProgressMark::NotStarted,
        pr_body: ProgressMark::NotStarted,
        ci,
        merge,
    }
}

fn apply_link_fallback(row: &mut PrProgressRow, link: Option<&TqGithubLink>) {
    let Some(link) = link else {
        return;
    };
    if row.number.is_none() {
        row.number = link.number;
    }
    if row.url.is_none() {
        row.url.clone_from(&link.url);
    }
}

fn pr_number_query(link: &TqGithubLink) -> Option<PrNumberQuery> {
    if !is_pull_request_link(link) {
        return None;
    }
    Some(PrNumberQuery {
        owner: link.owner.clone()?,
        repo: link.repo.clone()?,
        number: link.number?,
    })
}

fn is_pull_request_link(link: &TqGithubLink) -> bool {
    link.kind.as_deref() == Some("pull_request")
}

fn collect_pull_request_leaves<'a>(
    item: &'a TqChecklistItem,
    output: &mut Vec<&'a TqChecklistItem>,
) {
    if item.children.is_empty() {
        if pull_request_title(&item.content).is_some() {
            output.push(item);
        }
        return;
    }
    for child in &item.children {
        collect_pull_request_leaves(child, output);
    }
}

fn pull_request_title(content: &str) -> Option<String> {
    let item = content.strip_prefix("PR ")?;
    let (number, title) = item.split_once(": ")?;
    number.parse::<u32>().ok()?;
    (!title.is_empty()).then(|| title.to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::commands::agent::tui::progress::ProgressSelectionKey;
    use crate::commands::agent::tui::ui::test_support::create_test_session;
    use crate::commands::agent::types::SessionStatus;
    use crate::infra::github::{PrCiState, PrInfo, PrState};

    use super::*;

    fn checklist() -> TqChecklist {
        TqChecklist {
            id: "checklist-1".to_string(),
            name: "実装".to_string(),
            items: vec![
                TqChecklistItem {
                    id: "batch-1".to_string(),
                    content: "Batch 1".to_string(),
                    checked_at: None,
                    github_link_id: None,
                    children: vec![
                        TqChecklistItem {
                            id: "item-1".to_string(),
                            content: "PR 1: Add a sample feature".to_string(),
                            checked_at: None,
                            github_link_id: Some("link-1".to_string()),
                            children: Vec::new(),
                        },
                        TqChecklistItem {
                            id: "item-note".to_string(),
                            content: "Write a release note".to_string(),
                            checked_at: None,
                            github_link_id: None,
                            children: Vec::new(),
                        },
                    ],
                },
                TqChecklistItem {
                    id: "batch-2".to_string(),
                    content: "Batch 2".to_string(),
                    checked_at: None,
                    github_link_id: None,
                    children: vec![TqChecklistItem {
                        id: "item-2".to_string(),
                        content: "PR 2: Add another sample feature".to_string(),
                        checked_at: None,
                        github_link_id: None,
                        children: Vec::new(),
                    }],
                },
            ],
        }
    }

    fn github_link() -> TqGithubLink {
        TqGithubLink {
            id: "link-1".to_string(),
            kind: Some("pull_request".to_string()),
            owner: Some("sample-org".to_string()),
            repo: Some("sample-project".to_string()),
            number: Some(17),
            title: Some("Add a sample feature".to_string()),
            url: Some("https://github.com/sample-org/sample-project/pull/17".to_string()),
            state: Some("merged".to_string()),
        }
    }

    fn merged_progress() -> PrProgressInfo {
        PrProgressInfo {
            pull_request: PrInfo {
                number: 17,
                title: "Add a sample feature".to_string(),
                state: PrState::Merged,
                url: "https://github.com/sample-org/sample-project/pull/17".to_string(),
            },
            ci_state: PrCiState::Successful,
        }
    }

    #[test]
    fn resolves_linked_pull_requests_and_ignores_non_pr_checklist_items() {
        let checklist = checklist();
        let links = vec![github_link()];

        assert_eq!(
            (
                pull_request_queries(&checklist, &links),
                build_implementation_plan(
                    &checklist,
                    &links,
                    &[],
                    &HashMap::from([(
                        ("sample-org".to_string(), "sample-project".to_string(), 17),
                        Some(merged_progress()),
                    )]),
                    false,
                ),
            ),
            (
                vec![PrNumberQuery {
                    owner: "sample-org".to_string(),
                    repo: "sample-project".to_string(),
                    number: 17,
                }],
                ImplementationPlan {
                    batches: vec![
                        ImplementationBatch {
                            id: "batch-1".to_string(),
                            title: "Batch 1".to_string(),
                            items: vec![ImplementationItem {
                                id: "item-1".to_string(),
                                title: "Add a sample feature".to_string(),
                                delegate_session_id: None,
                                github_link: Some(github_link()),
                                pull_request: Some(merged_progress()),
                                lookup_unavailable: false,
                            }],
                        },
                        ImplementationBatch {
                            id: "batch-2".to_string(),
                            title: "Batch 2".to_string(),
                            items: vec![ImplementationItem {
                                id: "item-2".to_string(),
                                title: "Add another sample feature".to_string(),
                                delegate_session_id: None,
                                github_link: None,
                                pull_request: None,
                                lookup_unavailable: false,
                            }],
                        },
                    ],
                    unmatched_delegates: Vec::new(),
                },
            ),
        );
    }

    #[test]
    fn collapses_merged_batches_and_leaves_unstarted_rows_unselectable() {
        let plan = build_implementation_plan(
            &checklist(),
            &[github_link()],
            &[],
            &HashMap::from([(
                ("sample-org".to_string(), "sample-project".to_string(), 17),
                Some(merged_progress()),
            )]),
            false,
        );
        let rows = build_progress_rows(
            &create_test_session("selected"),
            &[],
            &plan,
            true,
            &HashMap::new(),
            &AgentConfig::default(),
        );

        assert_eq!(
            (
                rows.clone(),
                rows.iter()
                    .map(ProgressRow::selection_key)
                    .collect::<Vec<_>>(),
            ),
            (
                vec![
                    ProgressRow::Work(super::super::progress::WorkProgressRow {
                        session_id: "selected".to_string(),
                        role: "session".to_string(),
                        mark: ProgressMark::Working,
                        status_label: None,
                        note: None,
                        pane_id: None,
                    }),
                    ProgressRow::ImplementationHeader {
                        merged_pr_count: 1,
                        total_pr_count: 2,
                        collapsed: false,
                    },
                    ProgressRow::BatchHeader {
                        batch_id: "batch-1".to_string(),
                        title: "Batch 1".to_string(),
                        merged_pr_count: 1,
                        collapsed: true,
                    },
                    ProgressRow::BatchHeader {
                        batch_id: "batch-2".to_string(),
                        title: "Batch 2".to_string(),
                        merged_pr_count: 0,
                        collapsed: false,
                    },
                    ProgressRow::PlannedPullRequest {
                        batch_id: "batch-2".to_string(),
                        checklist_item_id: "item-2".to_string(),
                        title: "Add another sample feature".to_string(),
                    },
                ],
                vec![
                    Some(ProgressSelectionKey::Session("selected".to_string())),
                    Some(ProgressSelectionKey::Implementation),
                    Some(ProgressSelectionKey::Batch("batch-1".to_string())),
                    Some(ProgressSelectionKey::Batch("batch-2".to_string())),
                    None,
                ],
            ),
        );
    }

    #[test]
    fn keeps_delegates_without_a_matching_checklist_pr_as_regular_rows() {
        let mut delegate_session = create_test_session("unmapped-delegate");
        delegate_session.label = Some("Unplanned delegated work".to_string());
        let plan = build_implementation_plan(
            &checklist(),
            &[github_link()],
            &[FetchedDelegate {
                session: delegate_session.clone(),
                checklist_item_id: Some("unlisted-item".to_string()),
                pull_request: Some(PrProgressInfo {
                    pull_request: PrInfo {
                        number: 23,
                        title: "Unplanned delegated change".to_string(),
                        state: PrState::Open,
                        url: "https://github.com/sample-org/sample-project/pull/23".to_string(),
                    },
                    ci_state: PrCiState::InProgress,
                }),
                lookup_unavailable: false,
            }],
            &HashMap::new(),
            false,
        );

        assert_eq!(
            build_progress_rows(
                &create_test_session("selected"),
                &[delegate_session],
                &plan,
                false,
                &HashMap::new(),
                &AgentConfig::default(),
            ),
            vec![
                ProgressRow::Work(super::super::progress::WorkProgressRow {
                    session_id: "selected".to_string(),
                    role: "session".to_string(),
                    mark: ProgressMark::Working,
                    status_label: None,
                    note: None,
                    pane_id: None,
                }),
                ProgressRow::ImplementationHeader {
                    merged_pr_count: 1,
                    total_pr_count: 2,
                    collapsed: true,
                },
                ProgressRow::UnmatchedDelegatesHeader { collapsed: false },
                ProgressRow::PullRequest(super::super::progress::PrProgressRow {
                    session_id: "unmapped-delegate".to_string(),
                    title: "Unplanned delegated change".to_string(),
                    number: Some(23),
                    url: Some("https://github.com/sample-org/sample-project/pull/23".to_string()),
                    pane_id: None,
                    is_ended: false,
                    lookup_unavailable: false,
                    stages: StageMarks {
                        implementation: ProgressMark::Done,
                        diff_review: ProgressMark::Done,
                        pr_body: ProgressMark::Done,
                        ci: ProgressMark::Working,
                        merge: ProgressMark::Waiting,
                    },
                }),
            ],
        );
    }

    #[test]
    fn includes_new_unmapped_delegate_while_reusing_the_checklist_plan() {
        let plan =
            build_implementation_plan(&checklist(), &[github_link()], &[], &HashMap::new(), false);
        let mut delegate_session = create_test_session("new-delegate");
        delegate_session.label = Some("New delegated work".to_string());

        assert_eq!(
            build_progress_rows(
                &create_test_session("selected"),
                &[delegate_session],
                &plan,
                false,
                &HashMap::new(),
                &AgentConfig::default(),
            ),
            vec![
                ProgressRow::Work(super::super::progress::WorkProgressRow {
                    session_id: "selected".to_string(),
                    role: "session".to_string(),
                    mark: ProgressMark::Working,
                    status_label: None,
                    note: None,
                    pane_id: None,
                }),
                ProgressRow::ImplementationHeader {
                    merged_pr_count: 1,
                    total_pr_count: 2,
                    collapsed: true,
                },
                ProgressRow::UnmatchedDelegatesHeader { collapsed: false },
                ProgressRow::PullRequest(super::super::progress::PrProgressRow {
                    session_id: "new-delegate".to_string(),
                    title: "New delegated work".to_string(),
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
    fn renders_an_open_linked_pr_without_a_delegate_as_a_complete_row() {
        let checklist = TqChecklist {
            id: "checklist-1".to_string(),
            name: "実装".to_string(),
            items: vec![TqChecklistItem {
                id: "batch-1".to_string(),
                content: "Batch 1".to_string(),
                checked_at: None,
                github_link_id: None,
                children: vec![TqChecklistItem {
                    id: "item-1".to_string(),
                    content: "PR 1: Add a sample feature".to_string(),
                    checked_at: None,
                    github_link_id: Some("link-1".to_string()),
                    children: Vec::new(),
                }],
            }],
        };
        let github_link = TqGithubLink {
            state: Some("open".to_string()),
            ..github_link()
        };
        let pull_request = PrProgressInfo {
            pull_request: PrInfo {
                number: 17,
                title: "Add a sample feature".to_string(),
                state: PrState::Open,
                url: "https://github.com/sample-org/sample-project/pull/17".to_string(),
            },
            ci_state: PrCiState::Successful,
        };
        let plan = build_implementation_plan(
            &checklist,
            &[github_link],
            &[],
            &HashMap::from([(
                ("sample-org".to_string(), "sample-project".to_string(), 17),
                Some(pull_request),
            )]),
            false,
        );

        assert_eq!(
            build_progress_rows(
                &create_test_session("selected"),
                &[],
                &plan,
                true,
                &HashMap::new(),
                &AgentConfig::default(),
            ),
            vec![
                ProgressRow::Work(super::super::progress::WorkProgressRow {
                    session_id: "selected".to_string(),
                    role: "session".to_string(),
                    mark: ProgressMark::Working,
                    status_label: None,
                    note: None,
                    pane_id: None,
                }),
                ProgressRow::ImplementationHeader {
                    merged_pr_count: 0,
                    total_pr_count: 1,
                    collapsed: false,
                },
                ProgressRow::BatchHeader {
                    batch_id: "batch-1".to_string(),
                    title: "Batch 1".to_string(),
                    merged_pr_count: 0,
                    collapsed: false,
                },
                ProgressRow::ChecklistPullRequest {
                    batch_id: "batch-1".to_string(),
                    row: super::super::progress::PrProgressRow {
                        session_id: "checklist-item:item-1".to_string(),
                        title: "Add a sample feature".to_string(),
                        number: Some(17),
                        url: Some(
                            "https://github.com/sample-org/sample-project/pull/17".to_string(),
                        ),
                        pane_id: None,
                        is_ended: true,
                        lookup_unavailable: false,
                        stages: StageMarks {
                            implementation: ProgressMark::Done,
                            diff_review: ProgressMark::NotStarted,
                            pr_body: ProgressMark::NotStarted,
                            ci: ProgressMark::Done,
                            merge: ProgressMark::User,
                        },
                    },
                },
            ],
        );
    }

    #[test]
    fn includes_batch_heading_when_the_plan_has_one_batch() {
        let plan = ImplementationPlan {
            batches: vec![ImplementationBatch {
                id: "batch-1".to_string(),
                title: "Batch 1".to_string(),
                items: vec![ImplementationItem {
                    id: "item-1".to_string(),
                    title: "Add a sample feature".to_string(),
                    delegate_session_id: None,
                    github_link: None,
                    pull_request: None,
                    lookup_unavailable: false,
                }],
            }],
            unmatched_delegates: Vec::new(),
        };

        assert_eq!(
            build_progress_rows(
                &create_test_session("selected"),
                &[],
                &plan,
                true,
                &HashMap::new(),
                &AgentConfig::default(),
            ),
            vec![
                ProgressRow::Work(super::super::progress::WorkProgressRow {
                    session_id: "selected".to_string(),
                    role: "session".to_string(),
                    mark: ProgressMark::Working,
                    status_label: None,
                    note: None,
                    pane_id: None,
                }),
                ProgressRow::ImplementationHeader {
                    merged_pr_count: 0,
                    total_pr_count: 1,
                    collapsed: false,
                },
                ProgressRow::BatchHeader {
                    batch_id: "batch-1".to_string(),
                    title: "Batch 1".to_string(),
                    merged_pr_count: 0,
                    collapsed: false,
                },
                ProgressRow::PlannedPullRequest {
                    batch_id: "batch-1".to_string(),
                    checklist_item_id: "item-1".to_string(),
                    title: "Add a sample feature".to_string(),
                },
            ],
        );
    }

    #[test]
    fn keeps_an_empty_implementation_checklist_visible() {
        let checklist = TqChecklist {
            id: "checklist-1".to_string(),
            name: "実装".to_string(),
            items: Vec::new(),
        };
        let plan = build_implementation_plan(&checklist, &[], &[], &HashMap::new(), false);

        assert_eq!(
            build_progress_rows(
                &create_test_session("selected"),
                &[],
                &plan,
                true,
                &HashMap::new(),
                &AgentConfig::default(),
            ),
            vec![
                ProgressRow::Work(super::super::progress::WorkProgressRow {
                    session_id: "selected".to_string(),
                    role: "session".to_string(),
                    mark: ProgressMark::Working,
                    status_label: None,
                    note: None,
                    pane_id: None,
                }),
                ProgressRow::ImplementationHeader {
                    merged_pr_count: 0,
                    total_pr_count: 0,
                    collapsed: false,
                },
            ],
        );
    }

    #[test]
    fn collapses_a_fully_merged_single_batch_into_its_summary_row() {
        let plan = ImplementationPlan {
            batches: vec![ImplementationBatch {
                id: "batch-1".to_string(),
                title: "Batch 1".to_string(),
                items: vec![ImplementationItem {
                    id: "item-1".to_string(),
                    title: "Add a sample feature".to_string(),
                    delegate_session_id: None,
                    github_link: None,
                    pull_request: Some(merged_progress()),
                    lookup_unavailable: false,
                }],
            }],
            unmatched_delegates: Vec::new(),
        };

        assert_eq!(
            build_progress_rows(
                &create_test_session("selected"),
                &[],
                &plan,
                true,
                &HashMap::new(),
                &AgentConfig::default(),
            ),
            vec![
                ProgressRow::Work(super::super::progress::WorkProgressRow {
                    session_id: "selected".to_string(),
                    role: "session".to_string(),
                    mark: ProgressMark::Working,
                    status_label: None,
                    note: None,
                    pane_id: None,
                }),
                ProgressRow::ImplementationHeader {
                    merged_pr_count: 1,
                    total_pr_count: 1,
                    collapsed: false,
                },
                ProgressRow::BatchHeader {
                    batch_id: "batch-1".to_string(),
                    title: "Batch 1".to_string(),
                    merged_pr_count: 1,
                    collapsed: true,
                },
            ],
        );
    }

    #[test]
    fn omits_ended_unmatched_delegates_without_a_pull_request() {
        let mut delegate_session = create_test_session("ended-delegate");
        delegate_session.status = SessionStatus::Ended;
        let plan = build_implementation_plan(
            &checklist(),
            &[github_link()],
            &[FetchedDelegate {
                session: delegate_session.clone(),
                checklist_item_id: None,
                pull_request: None,
                lookup_unavailable: true,
            }],
            &HashMap::new(),
            false,
        );

        assert_eq!(
            build_progress_rows(
                &create_test_session("selected"),
                &[delegate_session],
                &plan,
                false,
                &HashMap::new(),
                &AgentConfig::default(),
            ),
            vec![
                ProgressRow::Work(super::super::progress::WorkProgressRow {
                    session_id: "selected".to_string(),
                    role: "session".to_string(),
                    mark: ProgressMark::Working,
                    status_label: None,
                    note: None,
                    pane_id: None,
                }),
                ProgressRow::ImplementationHeader {
                    merged_pr_count: 1,
                    total_pr_count: 2,
                    collapsed: true,
                },
            ],
        );
    }
}
