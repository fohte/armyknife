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
    let prs = if query_data.0.is_empty() {
        HashMap::new()
    } else {
        match GitHubClient::get() {
            Ok(client) => match client
                .get_pr_progress_for_branches_batch(&query_data.0)
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
                .1
                .get(&session.session_id)
                .and_then(|query| prs.get(&query.key()))
                .and_then(Option::as_ref)
                .cloned();
            FetchedDelegate {
                session,
                pull_request,
            }
        })
        .collect();

    Ok(FetchedProgress {
        selected_session_id,
        delegated,
        github_error,
    })
}

fn direct_delegates(sessions: Vec<Session>, parent_session_id: &str) -> Vec<Session> {
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

fn branch_queries_for_sessions(
    sessions: &[Session],
) -> (Vec<BranchPrQuery>, HashMap<String, BranchPrQuery>) {
    let mut queries = Vec::new();
    let mut by_session = HashMap::new();
    let mut seen = HashSet::new();

    for session in sessions {
        let Some(query) = branch_query_for(session) else {
            continue;
        };
        if seen.insert(query.key()) {
            queries.push(query.clone());
        }
        by_session.insert(session.session_id.clone(), query);
    }

    (queries, by_session)
}

fn branch_query_for(session: &Session) -> Option<BranchPrQuery> {
    let repo = GitRepo::open_at(&session.cwd).ok()?;
    if !repo.is_worktree() {
        return None;
    }
    let branch = repo.current_branch().ok()?;
    if branch == "HEAD" {
        return None;
    }
    let (owner, repo) = github_owner_and_repo(&repo).ok()?;
    Some(BranchPrQuery {
        owner,
        repo,
        branch,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;

    use chrono::{TimeZone, Utc};

    use super::direct_delegates;
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
