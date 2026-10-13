use std::collections::{HashMap, HashSet};

use crate::commands::agent::store;
use crate::commands::agent::types::Session;
use crate::infra::git::{GitRepo, github_owner_and_repo};
use crate::infra::github::{BranchPrQuery, GitHubClient};
use crate::infra::tq::{TqChecklist, TqClient, TqGithubLink};

use super::progress::{FetchedDelegate, FetchedProgress};
use super::progress_batches::{build_implementation_plan, pull_request_queries};

pub(super) async fn fetch_progress(
    selected_session_id: String,
    task_id: Option<String>,
) -> Result<FetchedProgress, String> {
    let sessions = tokio::task::spawn_blocking(store::list_all_sessions)
        .await
        .map_err(|error| format!("failed to read agent sessions: {error}"))?
        .map_err(|error| format!("failed to read agent sessions: {error}"))?;
    let delegated = direct_delegates(sessions, &selected_session_id);

    let query_data = tokio::task::spawn_blocking({
        let delegated = delegated.clone();
        move || branch_queries_for_sessions(&delegated)
    })
    .await
    .map_err(|error| format!("failed to inspect delegated worktrees: {error}"))?;

    let implementation_checklist = fetch_implementation_checklist(task_id.as_deref()).await;
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

    let delegated: Vec<FetchedDelegate> = delegated
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
        .collect();
    let implementation = implementation_checklist.map(|(checklist, github_links)| {
        build_implementation_plan(
            &checklist,
            &github_links,
            &delegated,
            &linked_prs,
            linked_pr_lookup_failed,
        )
    });

    Ok(FetchedProgress {
        selected_session_id,
        delegated,
        implementation,
        github_error,
    })
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

pub(super) fn direct_delegates(sessions: Vec<Session>, parent_session_id: &str) -> Vec<Session> {
    let mut delegated = sessions
        .into_iter()
        .filter(|session| {
            session.is_delegated
                && session.ancestor_session_ids.last().map(String::as_str)
                    == Some(parent_session_id)
        })
        .collect::<Vec<_>>();
    delegated.sort_by_key(|session| std::cmp::Reverse(session.updated_at));
    delegated
}

struct BranchQueries {
    queries: Vec<BranchPrQuery>,
    by_session: HashMap<String, BranchPrQuery>,
    checklist_item_id_by_session: HashMap<String, String>,
    unavailable: HashSet<String>,
}

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
    if let Some(context) = &session.pr_context {
        return BranchLookup::Query {
            query: BranchPrQuery {
                owner: context.owner.clone(),
                repo: context.repo.clone(),
                branch: context.branch.clone(),
            },
            checklist_item_id: context.checklist_item_id.clone(),
        };
    }

    let repo = match GitRepo::open_at(&session.cwd) {
        Ok(repo) => repo,
        Err(_) => return BranchLookup::Unavailable,
    };
    if !repo.is_worktree() {
        return BranchLookup::NotApplicable;
    }
    let branch = match repo.current_branch() {
        Ok(branch) => branch,
        Err(_) => return BranchLookup::Unavailable,
    };
    if branch == "HEAD" {
        return BranchLookup::NotApplicable;
    }
    let checklist_item_id = repo.local_config_value(&branch_checklist_item_config_key(&branch));
    match github_owner_and_repo(&repo) {
        Ok((owner, repo_name)) => BranchLookup::Query {
            query: BranchPrQuery {
                owner,
                repo: repo_name,
                branch,
            },
            checklist_item_id,
        },
        Err(_) => checklist_item_id
            .map(BranchLookup::ChecklistOnly)
            .unwrap_or(BranchLookup::Unavailable),
    }
}

fn branch_checklist_item_config_key(branch: &str) -> String {
    format!("branch.{branch}.x-tq-checklist-item-id")
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
    use std::path::PathBuf;

    use chrono::{TimeZone, Utc};

    use super::{branch_queries_for_sessions, direct_delegates};
    use crate::commands::agent::types::{Engine, Session, SessionPrContext, SessionStatus};
    use crate::infra::github::BranchPrQuery;

    #[test]
    fn lists_only_immediate_delegates_by_recent_activity() {
        let sessions = vec![
            session("direct-older", true, vec!["parent"], 10),
            session("nested", true, vec!["parent", "direct-newer"], 30),
            session("unrelated", true, vec!["other"], 40),
            session("direct-newer", true, vec!["parent"], 20),
            session("not-delegated", false, vec!["parent"], 50),
        ];

        assert_eq!(
            direct_delegates(sessions, "parent")
                .into_iter()
                .map(|session| session.session_id)
                .collect::<Vec<_>>(),
            vec!["direct-newer", "direct-older"],
        );
    }

    #[test]
    fn marks_unresolvable_worktrees_unavailable_without_creating_pr_queries() {
        let mut delegate = session("delegate", true, vec!["parent"], 10);
        delegate.cwd = std::env::temp_dir()
            .join(format!(
                "armyknife-test-missing-repository-{}",
                std::process::id()
            ))
            .join("worktree");
        let queries = branch_queries_for_sessions(&[delegate]);

        assert_eq!(
            (queries.queries, queries.by_session, queries.unavailable,),
            (
                Vec::new(),
                HashMap::new(),
                HashSet::from(["delegate".to_string()]),
            ),
        );
    }

    #[test]
    fn uses_saved_pr_context_after_worktree_removal() {
        let missing_cwd = std::env::temp_dir()
            .join(format!(
                "armyknife-test-missing-pr-worktree-{}",
                std::process::id()
            ))
            .join("worktree");
        let mut recorded = session("recorded", true, vec!["parent"], 20);
        recorded.cwd = missing_cwd;
        recorded.pr_context = Some(SessionPrContext {
            owner: "sample-owner".to_string(),
            repo: "sample-repo".to_string(),
            branch: "sample-branch".to_string(),
            checklist_item_id: Some("sample-item".to_string()),
        });

        let queries = branch_queries_for_sessions(&[recorded]);
        let query = BranchPrQuery {
            owner: "sample-owner".to_string(),
            repo: "sample-repo".to_string(),
            branch: "sample-branch".to_string(),
        };

        assert_eq!(
            (
                queries.queries,
                queries.by_session,
                queries.checklist_item_id_by_session,
                queries.unavailable,
            ),
            (
                vec![query.clone()],
                HashMap::from([("recorded".to_string(), query)]),
                HashMap::from([("recorded".to_string(), "sample-item".to_string())]),
                HashSet::new(),
            ),
        );
    }

    fn session(
        session_id: &str,
        is_delegated: bool,
        ancestor_session_ids: Vec<&str>,
        updated_second: i64,
    ) -> Session {
        let updated_at = Utc.timestamp_opt(updated_second, 0).single().unwrap();
        Session {
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
            work_type_pinned: false,
            is_delegated,
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
        }
    }
}
