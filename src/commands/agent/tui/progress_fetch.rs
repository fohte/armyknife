use std::collections::{HashMap, HashSet};

use crate::commands::agent::store;
use crate::commands::agent::types::{Session, SessionPrContext};
use crate::infra::git::GitRepo;
use crate::infra::github::{BranchPrQuery, GitHubClient, PrProgressInfo};
use crate::infra::tq::{TqChecklist, TqClient, TqGithubLink};

use super::super::session_pr_context::{self, Resolution};
use super::progress::{FetchedDelegate, FetchedProgress, ProgressFetchScope};
use super::progress_batches::{build_implementation_plan_with_work_sessions, pull_request_queries};

pub(super) async fn fetch_worker_progress(
    sessions: Vec<Session>,
) -> Result<Vec<FetchedDelegate>, String> {
    let query_sessions = sessions.clone();
    let query_data =
        tokio::task::spawn_blocking(move || branch_queries_for_sessions(&query_sessions))
            .await
            .map_err(|error| format!("failed to inspect session worktrees: {error}"))?;

    let prs = if query_data.queries.is_empty() {
        HashMap::new()
    } else {
        let client =
            GitHubClient::get().map_err(|error| format!("GitHub is unavailable: {error:#}"))?;
        client
            .get_pr_progress_for_branches_batch(&query_data.queries)
            .await
            .map_err(|error| format!("failed to fetch PR progress: {error:#}"))?
    };

    Ok(fetched_sessions(sessions, &query_data, &prs))
}

pub(super) async fn fetch_progress(
    selected_session_id: String,
    request_scope: ProgressFetchScope,
) -> Result<FetchedProgress, String> {
    let sessions = tokio::task::spawn_blocking(store::list_all_sessions)
        .await
        .map_err(|error| format!("failed to read agent sessions: {error}"))?
        .map_err(|error| format!("failed to read agent sessions: {error}"))?;
    let work_sessions = work_sessions_for_scope(&sessions, &request_scope);
    let delegated = match request_scope.session_ids.as_ref() {
        Some(session_ids) => {
            let session_ids = session_ids.iter().cloned().collect::<HashSet<_>>();
            let other_task_session_ids = request_scope
                .other_task_session_ids
                .iter()
                .cloned()
                .collect::<HashSet<_>>();
            task_descendant_children(&sessions, &session_ids, &other_task_session_ids)
        }
        None => direct_children(sessions, &selected_session_id),
    };
    let mut sessions_with_progress = delegated.clone();
    sessions_with_progress.extend(work_sessions.iter().cloned());

    let query_data = tokio::task::spawn_blocking({
        move || branch_queries_for_sessions(&sessions_with_progress)
    })
    .await
    .map_err(|error| format!("failed to inspect delegated worktrees: {error}"))?;

    let implementation_checklist =
        fetch_implementation_checklist(request_scope.task_id.as_deref()).await;
    let linked_pr_queries = implementation_checklist
        .as_ref()
        .map(|(checklist, github_links)| pull_request_queries(checklist, github_links))
        .unwrap_or_default();

    let mut github_error = None;
    let mut linked_pr_lookup_failed = false;
    let mut prs = HashMap::new();
    let mut linked_prs = HashMap::new();
    if !query_data.queries.is_empty() || !linked_pr_queries.is_empty() {
        match GitHubClient::get() {
            Ok(client) => {
                if !query_data.queries.is_empty() {
                    match client
                        .get_pr_progress_for_branches_batch(&query_data.queries)
                        .await
                    {
                        Ok(found) => prs = found,
                        Err(error) => {
                            github_error = Some(format!("failed to fetch PR progress: {error:#}"));
                        }
                    }
                }
                if !linked_pr_queries.is_empty() {
                    match client
                        .get_pr_progress_for_numbers_batch(&linked_pr_queries)
                        .await
                    {
                        Ok(found) => linked_prs = found,
                        Err(error) => {
                            linked_pr_lookup_failed = true;
                            github_error.get_or_insert_with(|| {
                                format!("failed to fetch linked PR progress: {error:#}")
                            });
                        }
                    }
                }
            }
            Err(error) => {
                linked_pr_lookup_failed = !linked_pr_queries.is_empty();
                github_error = Some(format!("GitHub is unavailable: {error:#}"));
            }
        }
    }

    let fetched_delegated = fetched_sessions(delegated, &query_data, &prs);
    let fetched_work_sessions = fetched_sessions(work_sessions, &query_data, &prs);
    let implementation = implementation_checklist.map(|(checklist, github_links)| {
        build_implementation_plan_with_work_sessions(
            &checklist,
            &github_links,
            &fetched_delegated,
            &fetched_work_sessions,
            &linked_prs,
            linked_pr_lookup_failed,
        )
    });

    Ok(FetchedProgress {
        selected_session_id,
        request_scope: Some(request_scope),
        delegated: fetched_delegated,
        implementation,
        github_error,
    })
}

fn work_sessions_for_scope(
    sessions: &[Session],
    request_scope: &ProgressFetchScope,
) -> Vec<Session> {
    let Some(session_ids) = request_scope.session_ids.as_ref() else {
        return Vec::new();
    };
    let session_ids = session_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    sessions
        .iter()
        .filter(|session| session_ids.contains(session.session_id.as_str()))
        .cloned()
        .collect()
}

fn fetched_sessions(
    sessions: Vec<Session>,
    query_data: &BranchQueries,
    prs: &HashMap<(String, String, String), Option<PrProgressInfo>>,
) -> Vec<FetchedDelegate> {
    sessions
        .into_iter()
        .map(|session| {
            let pull_request = query_data
                .by_session
                .get(&session.session_id)
                .and_then(|query| prs.get(&query.key()))
                .and_then(Option::as_ref)
                .cloned();
            let lookup_unavailable = query_data.unavailable.contains(&session.session_id);
            let checklist_item_id = query_data
                .checklist_item_id_by_session
                .get(&session.session_id)
                .cloned();
            FetchedDelegate {
                session,
                checklist_item_id,
                pull_request,
                lookup_unavailable,
            }
        })
        .collect()
}

// Checklist metadata is optional so tq outages keep the existing delegate view available.
async fn fetch_implementation_checklist(
    task_id: Option<&str>,
) -> Option<(TqChecklist, Vec<TqGithubLink>)> {
    let task_id = task_id?;
    let client = TqClient::detect()?;
    let checklist = client
        .list_checklists(task_id)
        .await
        .ok()?
        .into_iter()
        .find(|checklist| checklist.name == "実装")?;
    let github_links = client.task_github_links(task_id).await.ok()?;
    Some((checklist, github_links))
}

pub(super) fn direct_children(sessions: Vec<Session>, parent_session_id: &str) -> Vec<Session> {
    let mut children = sessions
        .into_iter()
        .filter(|session| {
            session.ancestor_session_ids.last().map(String::as_str) == Some(parent_session_id)
        })
        .collect::<Vec<_>>();
    children.sort_by_key(|session| std::cmp::Reverse(session.updated_at));
    children
}

pub(super) fn task_descendant_children(
    sessions: &[Session],
    task_session_ids: &HashSet<String>,
    other_task_session_ids: &HashSet<String>,
) -> Vec<Session> {
    let sessions_by_id = sessions
        .iter()
        .map(|session| (session.session_id.as_str(), session))
        .collect::<HashMap<_, _>>();
    let mut children = Vec::new();
    for session in sessions {
        if task_session_ids.contains(&session.session_id)
            || other_task_session_ids.contains(&session.session_id)
        {
            continue;
        }

        let mut task_ancestor = None;
        for ancestor_id in session.ancestor_session_ids.iter().rev() {
            if task_session_ids.contains(ancestor_id)
                || other_task_session_ids.contains(ancestor_id)
            {
                task_ancestor = Some(ancestor_id);
                break;
            }
            if !sessions_by_id.contains_key(ancestor_id.as_str()) {
                break;
            }
        }
        if task_ancestor.is_some_and(|ancestor_id| task_session_ids.contains(ancestor_id)) {
            children.push(session.clone());
        }
    }
    children.sort_by_key(|session| std::cmp::Reverse(session.updated_at));
    children
}

struct BranchQueries {
    queries: Vec<BranchPrQuery>,
    by_session: HashMap<String, BranchPrQuery>,
    checklist_item_id_by_session: HashMap<String, String>,
    unavailable: HashSet<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum BranchLookup {
    Query {
        query: BranchPrQuery,
        checklist_item_id: Option<String>,
    },
    ChecklistOnly(String),
    NotApplicable,
    Unavailable,
}

fn branch_queries_for_sessions(sessions: &[Session]) -> BranchQueries {
    let mut result = BranchQueries {
        queries: Vec::new(),
        by_session: HashMap::new(),
        checklist_item_id_by_session: HashMap::new(),
        unavailable: HashSet::new(),
    };
    let mut seen = HashSet::new();

    for session in sessions {
        let query = match branch_query_for(session) {
            BranchLookup::Query {
                query,
                checklist_item_id,
            } => {
                if let Some(item_id) = checklist_item_id {
                    result
                        .checklist_item_id_by_session
                        .insert(session.session_id.clone(), item_id);
                }
                query
            }
            BranchLookup::ChecklistOnly(item_id) => {
                result
                    .checklist_item_id_by_session
                    .insert(session.session_id.clone(), item_id);
                result.unavailable.insert(session.session_id.clone());
                continue;
            }
            BranchLookup::NotApplicable => continue,
            BranchLookup::Unavailable => {
                result.unavailable.insert(session.session_id.clone());
                continue;
            }
        };
        if seen.insert(query.key()) {
            result.queries.push(query.clone());
        }
        result.by_session.insert(session.session_id.clone(), query);
    }

    result
}

fn branch_query_for(session: &Session) -> BranchLookup {
    branch_query_for_with(session, || {
        GitRepo::open_at(&session.cwd)
            .ok()
            .map(|repo| session_pr_context::resolve(&repo))
    })
}

fn branch_query_for_with(
    session: &Session,
    resolve_live: impl FnOnce() -> Option<Resolution>,
) -> BranchLookup {
    if let Some(context) = &session.pr_context {
        return branch_query_from_context(context.clone());
    }

    branch_lookup_for_resolution(resolve_live())
}

fn branch_lookup_for_resolution(live_resolution: Option<Resolution>) -> BranchLookup {
    match live_resolution {
        Some(Resolution::Context(context)) => branch_query_from_context(context),
        Some(Resolution::ChecklistOnly(item_id)) => BranchLookup::ChecklistOnly(item_id),
        Some(Resolution::NotApplicable) => BranchLookup::NotApplicable,
        Some(Resolution::Unavailable) => BranchLookup::Unavailable,
        None => BranchLookup::Unavailable,
    }
}

fn branch_query_from_context(context: SessionPrContext) -> BranchLookup {
    BranchLookup::Query {
        query: BranchPrQuery {
            owner: context.owner,
            repo: context.repo,
            branch: context.branch,
        },
        checklist_item_id: context.checklist_item_id,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
    use std::path::PathBuf;

    use chrono::{TimeZone, Utc};
    use rstest::{fixture, rstest};

    use super::{
        BranchLookup, Resolution, branch_queries_for_sessions, branch_query_for_with,
        direct_children, task_descendant_children, work_sessions_for_scope,
    };
    use crate::commands::agent::tui::progress::ProgressFetchScope;
    use crate::commands::agent::types::{Engine, Session, SessionPrContext, SessionStatus};
    use crate::infra::github::BranchPrQuery;

    #[test]
    fn lists_all_immediate_children_by_recent_activity() {
        let mut denied_child = session("denied-child", vec!["parent"], 40);
        denied_child.child_launch = crate::commands::agent::types::ChildLaunchPermission::Deny;
        let sessions = vec![
            session("direct-older", vec!["parent"], 10),
            session("nested", vec!["parent", "direct-newer"], 30),
            session("unrelated", vec!["other"], 60),
            session("direct-newer", vec!["parent"], 20),
            denied_child,
        ];

        assert_eq!(
            direct_children(sessions, "parent")
                .into_iter()
                .map(|session| session.session_id)
                .collect::<Vec<_>>(),
            vec!["denied-child", "direct-newer", "direct-older"],
        );
    }

    #[test]
    fn lists_task_descendant_children_without_crossing_into_another_task() {
        let mut direct_child = session("direct-child", vec!["task-root"], 4);
        direct_child.child_launch = crate::commands::agent::types::ChildLaunchPermission::Deny;
        let sessions = vec![
            session("task-root", vec![], 1),
            session("same-task-child", vec!["task-root"], 2),
            session("other-task-child", vec!["task-root"], 3),
            direct_child,
            session(
                "nested-child",
                vec!["task-root", "same-task-child", "direct-child"],
                5,
            ),
            session(
                "other-task-child-session",
                vec!["task-root", "other-task-child"],
                6,
            ),
            session("orphan-child", vec!["task-root", "removed-task-child"], 7),
        ];

        assert_eq!(
            task_descendant_children(
                &sessions,
                &HashSet::from(["task-root".to_string(), "same-task-child".to_string()]),
                &HashSet::from(["other-task-child".to_string()]),
            )
            .into_iter()
            .map(|session| session.session_id)
            .collect::<Vec<_>>(),
            vec!["nested-child", "direct-child"],
        );
    }

    #[test]
    fn includes_same_task_worker_checklist_metadata_in_progress_candidates() {
        let mut design = session("design", vec![], 1);
        design.pr_context = Some(SessionPrContext {
            owner: "sample-owner".to_string(),
            repo: "sample-repo".to_string(),
            branch: "design-branch".to_string(),
            checklist_item_id: None,
        });
        let mut worker = session("worker", vec!["design"], 2);
        worker.pr_context = Some(SessionPrContext {
            owner: "sample-owner".to_string(),
            repo: "sample-repo".to_string(),
            branch: "worker-branch".to_string(),
            checklist_item_id: Some("planned-item".to_string()),
        });
        let mut unrelated = session("unrelated", vec![], 3);
        unrelated.pr_context = Some(SessionPrContext {
            owner: "other-owner".to_string(),
            repo: "other-repo".to_string(),
            branch: "other-branch".to_string(),
            checklist_item_id: Some("other-item".to_string()),
        });
        let sessions = vec![design, worker, unrelated];
        let request_scope = ProgressFetchScope {
            task_id: Some("task".to_string()),
            session_ids: Some(vec!["design".to_string(), "worker".to_string()]),
            other_task_session_ids: Vec::new(),
        };
        let work_sessions = work_sessions_for_scope(&sessions, &request_scope);
        let queries = branch_queries_for_sessions(&work_sessions);

        assert_eq!(
            (
                work_sessions
                    .iter()
                    .map(|session| session.session_id.as_str())
                    .collect::<Vec<_>>(),
                queries.checklist_item_id_by_session,
            ),
            (
                vec!["design", "worker"],
                HashMap::from([("worker".to_string(), "planned-item".to_string())]),
            ),
        );
    }

    #[test]
    fn batches_worker_branches_across_task_roots_and_preserves_checklist_ids() {
        let mut first = session("first-worker", vec!["first-task"], 1);
        first.pr_context = Some(SessionPrContext {
            owner: "sample-owner".to_string(),
            repo: "sample-repo".to_string(),
            branch: "first-branch".to_string(),
            checklist_item_id: Some("first-item".to_string()),
        });
        let mut second = session("second-worker", vec!["second-task"], 2);
        second.pr_context = Some(SessionPrContext {
            owner: "sample-owner".to_string(),
            repo: "sample-repo".to_string(),
            branch: "second-branch".to_string(),
            checklist_item_id: Some("second-item".to_string()),
        });
        let mut duplicate = session("duplicate-worker", vec!["third-task"], 3);
        duplicate.pr_context = Some(SessionPrContext {
            owner: "sample-owner".to_string(),
            repo: "sample-repo".to_string(),
            branch: "first-branch".to_string(),
            checklist_item_id: Some("duplicate-item".to_string()),
        });

        let queries = branch_queries_for_sessions(&[first, second, duplicate]);

        assert_eq!(
            (
                queries.queries,
                queries.by_session.len(),
                queries.checklist_item_id_by_session,
            ),
            (
                vec![
                    BranchPrQuery {
                        owner: "sample-owner".to_string(),
                        repo: "sample-repo".to_string(),
                        branch: "first-branch".to_string(),
                    },
                    BranchPrQuery {
                        owner: "sample-owner".to_string(),
                        repo: "sample-repo".to_string(),
                        branch: "second-branch".to_string(),
                    },
                ],
                3,
                HashMap::from([
                    ("first-worker".to_string(), "first-item".to_string()),
                    ("second-worker".to_string(), "second-item".to_string()),
                    ("duplicate-worker".to_string(), "duplicate-item".to_string()),
                ]),
            ),
        );
    }

    #[fixture]
    fn saved_context() -> SessionPrContext {
        SessionPrContext {
            owner: "sample-owner".to_string(),
            repo: "sample-repo".to_string(),
            branch: "saved-branch".to_string(),
            checklist_item_id: Some("saved-item".to_string()),
        }
    }

    #[rstest]
    #[case::missing_repository(None, BranchLookup::Unavailable)]
    #[case::unavailable_repository(Some(Resolution::Unavailable), BranchLookup::Unavailable)]
    #[case::live_context(
        Some(Resolution::Context(SessionPrContext {
            owner: "sample-owner".to_string(),
            repo: "sample-repo".to_string(),
            branch: "live-branch".to_string(),
            checklist_item_id: Some("live-item".to_string()),
        })),
        BranchLookup::Query {
            query: BranchPrQuery {
                owner: "sample-owner".to_string(),
                repo: "sample-repo".to_string(),
                branch: "live-branch".to_string(),
            },
            checklist_item_id: Some("live-item".to_string()),
        },
    )]
    #[case::checklist_only(
        Some(Resolution::ChecklistOnly("live-item".to_string())),
        BranchLookup::ChecklistOnly("live-item".to_string()),
    )]
    #[case::not_applicable(Some(Resolution::NotApplicable), BranchLookup::NotApplicable)]
    fn uses_live_resolution_without_saved_context(
        #[case] live_resolution: Option<Resolution>,
        #[case] expected: BranchLookup,
    ) {
        let session = session("child", vec!["parent"], 10);
        assert_eq!(
            branch_query_for_with(&session, || live_resolution),
            expected
        );
    }

    #[rstest]
    fn prefers_saved_context_over_live_context(saved_context: SessionPrContext) {
        let mut session = session("child", vec!["parent"], 10);
        session.pr_context = Some(saved_context);
        let resolver_called = Cell::new(false);

        assert_eq!(
            (
                branch_query_for_with(&session, || {
                    resolver_called.set(true);
                    Some(Resolution::Context(SessionPrContext {
                        owner: "other-owner".to_string(),
                        repo: "other-repo".to_string(),
                        branch: "live-branch".to_string(),
                        checklist_item_id: Some("live-item".to_string()),
                    }))
                }),
                resolver_called.get(),
            ),
            (
                BranchLookup::Query {
                    query: BranchPrQuery {
                        owner: "sample-owner".to_string(),
                        repo: "sample-repo".to_string(),
                        branch: "saved-branch".to_string(),
                    },
                    checklist_item_id: Some("saved-item".to_string()),
                },
                false,
            ),
        );
    }

    fn session(session_id: &str, ancestor_session_ids: Vec<&str>, updated_second: i64) -> Session {
        let updated_at = Utc.timestamp_opt(updated_second, 0).single().unwrap();
        Session {
            archived_at: None,
            pr_context: None,
            session_id: session_id.to_string(),
            crit_urls: Vec::new(),
            pending_human_review_ids: BTreeSet::new(),
            cwd: PathBuf::from("/tmp/example-project"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            agent_status: None,
            agent_status_clear_pending: false,
            created_at: updated_at,
            updated_at,
            last_message: None,
            current_tool: None,
            label: None,
            work_type: None,
            vibe_enabled: false,
            launched_with_vibe: false,
            work_type_pinned: false,
            child_launch: Default::default(),
            ancestor_session_ids: ancestor_session_ids
                .into_iter()
                .map(str::to_owned)
                .collect(),
            pending_bg_task_ids: BTreeSet::new(),
            pending_agent_task_ids: BTreeSet::new(),
            pending_permission_agent_ids: BTreeSet::new(),
            pending_permission_request_ids: BTreeMap::new(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            engine: Engine::Claude,
            model: None,
            effort: None,
        }
    }
}
