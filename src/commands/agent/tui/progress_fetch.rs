use std::collections::{HashMap, HashSet};

use crate::commands::agent::store;
use crate::commands::agent::types::Session;
use crate::infra::git::{GitRepo, github_owner_and_repo};
use crate::infra::github::{BranchPrQuery, GitHubClient};

use super::progress::{FetchedDelegate, FetchedProgress};

pub(super) async fn fetch_progress(selected_session_id: String) -> Result<FetchedProgress, String> {
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

    let mut github_error = None;
    let prs = if query_data.queries.is_empty() {
        HashMap::new()
    } else {
        match GitHubClient::get() {
            Ok(client) => match client
                .get_pr_progress_for_branches_batch(&query_data.queries)
                .await
            {
                Ok(prs) => prs,
                Err(error) => {
                    github_error = Some(format!("failed to fetch PR progress: {error:#}"));
                    HashMap::new()
                }
            },
            Err(error) => {
                github_error = Some(format!("GitHub is unavailable: {error:#}"));
                HashMap::new()
            }
        }
    };

    let delegated = delegated
        .into_iter()
        .map(|session| {
            let pull_request = query_data
                .by_session
                .get(&session.session_id)
                .and_then(|query| prs.get(&query.key()))
                .and_then(Option::as_ref)
                .cloned();
            let lookup_unavailable = query_data.unavailable.contains(&session.session_id);
            FetchedDelegate {
                session,
                pull_request,
                lookup_unavailable,
            }
        })
        .collect();

    Ok(FetchedProgress {
        selected_session_id,
        delegated,
        github_error,
    })
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
    unavailable: HashSet<String>,
}

enum BranchLookup {
    Query(BranchPrQuery),
    NotApplicable,
    Unavailable,
}

fn branch_queries_for_sessions(sessions: &[Session]) -> BranchQueries {
    let mut result = BranchQueries {
        queries: Vec::new(),
        by_session: HashMap::new(),
        unavailable: HashSet::new(),
    };
    let mut seen = HashSet::new();

    for session in sessions {
        let query = match branch_query_for(session) {
            BranchLookup::Query(query) => query,
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
    let (owner, repo) = match github_owner_and_repo(&repo) {
        Ok(remote) => remote,
        Err(_) => return BranchLookup::Unavailable,
    };
    BranchLookup::Query(BranchPrQuery {
        owner,
        repo,
        branch,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
    use std::path::PathBuf;

    use chrono::{TimeZone, Utc};

    use super::{branch_queries_for_sessions, direct_delegates};
    use crate::commands::agent::types::{Engine, Session, SessionStatus};

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

    fn session(
        session_id: &str,
        is_delegated: bool,
        ancestor_session_ids: Vec<&str>,
        updated_second: i64,
    ) -> Session {
        let updated_at = Utc.timestamp_opt(updated_second, 0).single().unwrap();
        Session {
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
