use crate::commands::agent::types::{Session, SessionPrContext};
use crate::infra::github::{BranchPrQuery, GitHubClient, PrNumberQuery, PrProgressInfo};
use crate::infra::tq::TqGithubLink;
use std::collections::{HashMap, HashSet};

use super::{
    CacheOperations, PullRequestKey, SessionEnrichmentCache, SessionPullRequestInfo,
    SessionTaskMetadata,
    cache::{merge_pull_request_results, prune_pull_request_results},
    pull_request_can_refresh,
};

impl SessionEnrichmentCache {
    pub(in crate::commands::agent::watch_web) async fn pull_requests(
        &self,
        sessions: &[Session],
        task_metadata: &SessionTaskMetadata,
    ) -> HashMap<String, Vec<SessionPullRequestInfo>> {
        let task_queries = linked_task_pr_queries(sessions, task_metadata);
        let number_queries = number_pr_queries(sessions, &task_queries);
        let query_keys = pull_request_keys(sessions, &number_queries);
        if query_keys.is_empty() {
            return HashMap::new();
        }

        let fetcher = std::sync::Arc::clone(&self.pull_request_fetcher);
        let cached = self
            .pull_requests
            .get_or_refresh(
                "GitHub",
                query_keys,
                None,
                {
                    let sessions = sessions.to_vec();
                    move |_, refresh_keys| {
                        let branch_keys = refresh_keys
                            .iter()
                            .filter_map(|key| match key {
                                PullRequestKey::Branch(key) => Some(key.clone()),
                                PullRequestKey::Number(_) => None,
                            })
                            .collect::<HashSet<_>>();
                        let number_keys = refresh_keys
                            .iter()
                            .filter_map(|key| match key {
                                PullRequestKey::Branch(_) => None,
                                PullRequestKey::Number(key) => Some(key.clone()),
                            })
                            .collect::<HashSet<_>>();
                        let sessions = sessions
                            .into_iter()
                            .filter(|session| {
                                session.pr_context.as_ref().is_some_and(|context| {
                                    branch_keys.contains(&(
                                        context.owner.clone(),
                                        context.repo.clone(),
                                        context.branch.clone(),
                                    ))
                                })
                            })
                            .collect();
                        let number_queries = number_queries
                            .into_iter()
                            .filter(|query| number_keys.contains(&query.key()))
                            .collect();
                        fetcher(sessions, number_queries)
                    }
                },
                CacheOperations {
                    merge: merge_pull_request_results,
                    prune: prune_pull_request_results,
                    can_refresh: pull_request_can_refresh,
                    fetched_keys: |_, refresh_keys| refresh_keys.clone(),
                },
            )
            .await;

        sessions
            .iter()
            .filter(|session| {
                session.pr_context.is_some()
                    || !session.created_pr_urls.is_empty()
                    || !linked_task_pr_queries_for_session(session, task_metadata).is_empty()
            })
            .map(|session| {
                let mut seen_pull_requests = HashSet::new();
                let mut pull_requests = Vec::new();
                if let Some(context) = &session.pr_context {
                    let key = PullRequestKey::Branch(branch_query(context).key());
                    if let Some(pull_request) = cached.get(&key).cloned().flatten() {
                        let pull_request_key = normalized_pull_request_key(
                            &context.owner,
                            &context.repo,
                            pull_request.pull_request.number,
                        );
                        if seen_pull_requests.insert(pull_request_key) {
                            pull_requests.push(SessionPullRequestInfo {
                                owner: context.owner.clone(),
                                repo: context.repo.clone(),
                                progress: pull_request,
                                from_cwd_branch: true,
                            });
                        }
                    }
                }
                append_numbered_pull_requests(
                    submitted_pr_queries(&session.created_pr_urls),
                    &cached,
                    &mut seen_pull_requests,
                    &mut pull_requests,
                );
                append_numbered_pull_requests(
                    linked_task_pr_queries_for_session(session, task_metadata),
                    &cached,
                    &mut seen_pull_requests,
                    &mut pull_requests,
                );
                (session.session_id.clone(), pull_requests)
            })
            .collect()
    }
}

fn append_numbered_pull_requests(
    queries: impl IntoIterator<Item = PrNumberQuery>,
    cached: &HashMap<PullRequestKey, Option<PrProgressInfo>>,
    seen_pull_requests: &mut HashSet<(String, String, u64)>,
    pull_requests: &mut Vec<SessionPullRequestInfo>,
) {
    for query in queries {
        let Some(pull_request) = cached
            .get(&PullRequestKey::Number(query.key()))
            .cloned()
            .flatten()
        else {
            continue;
        };
        if seen_pull_requests.insert(normalized_pull_request_key(
            &query.owner,
            &query.repo,
            query.number,
        )) {
            pull_requests.push(SessionPullRequestInfo {
                owner: query.owner,
                repo: query.repo,
                progress: pull_request,
                from_cwd_branch: false,
            });
        }
    }
}

fn normalized_pull_request_key(owner: &str, repo: &str, number: u64) -> (String, String, u64) {
    (
        owner.to_ascii_lowercase(),
        repo.to_ascii_lowercase(),
        number,
    )
}

fn pull_request_keys(
    sessions: &[Session],
    number_queries: &[PrNumberQuery],
) -> HashSet<PullRequestKey> {
    let mut keys = branch_pr_queries(sessions)
        .into_iter()
        .map(|query| PullRequestKey::Branch(query.key()))
        .collect::<HashSet<_>>();
    keys.extend(
        number_queries
            .iter()
            .map(|query| PullRequestKey::Number(query.key())),
    );
    keys
}

fn number_pr_queries(sessions: &[Session], task_queries: &[PrNumberQuery]) -> Vec<PrNumberQuery> {
    let mut queries = submitted_pr_queries_for_sessions(sessions);
    let mut seen = queries
        .iter()
        .map(PrNumberQuery::key)
        .collect::<HashSet<_>>();
    for query in task_queries {
        if seen.insert(query.key()) {
            queries.push(query.clone());
        }
    }
    queries
}

fn linked_task_pr_queries(
    sessions: &[Session],
    task_metadata: &SessionTaskMetadata,
) -> Vec<PrNumberQuery> {
    let mut queries = Vec::new();
    let mut seen = HashSet::new();
    for session in sessions {
        for query in linked_task_pr_queries_for_session(session, task_metadata) {
            if seen.insert(query.key()) {
                queries.push(query);
            }
        }
    }
    queries
}

fn linked_task_pr_queries_for_session(
    session: &Session,
    task_metadata: &SessionTaskMetadata,
) -> Vec<PrNumberQuery> {
    let mut seen = HashSet::new();
    task_metadata
        .links
        .get(&session.session_id)
        .into_iter()
        .flatten()
        .filter_map(|task| task_metadata.github_links.get(&task.task_id))
        .flatten()
        .filter_map(task_pr_query)
        .filter(|query| seen.insert(query.key()))
        .collect()
}

fn task_pr_query(link: &TqGithubLink) -> Option<PrNumberQuery> {
    match link.kind.as_deref() {
        Some("pull_request") => match (&link.owner, &link.repo, link.number) {
            (Some(owner), Some(repo), Some(number))
                if !owner.is_empty() && !repo.is_empty() && number > 0 =>
            {
                Some(PrNumberQuery {
                    owner: owner.clone(),
                    repo: repo.clone(),
                    number,
                })
            }
            _ => link.url.as_deref().and_then(PrNumberQuery::from_github_url),
        },
        Some(_) => None,
        None => link.url.as_deref().and_then(PrNumberQuery::from_github_url),
    }
}

fn branch_pr_queries(sessions: &[Session]) -> Vec<BranchPrQuery> {
    let mut queries = Vec::new();
    let mut seen = HashSet::new();
    for session in sessions {
        if let Some(context) = &session.pr_context {
            let query = branch_query(context);
            if seen.insert(query.key()) {
                queries.push(query);
            }
        }
    }
    queries
}

fn branch_query(context: &SessionPrContext) -> BranchPrQuery {
    BranchPrQuery {
        owner: context.owner.clone(),
        repo: context.repo.clone(),
        branch: context.branch.clone(),
    }
}

fn submitted_pr_queries_for_sessions(sessions: &[Session]) -> Vec<PrNumberQuery> {
    let mut queries = Vec::new();
    let mut seen = HashSet::new();
    for session in sessions {
        for query in submitted_pr_queries(&session.created_pr_urls) {
            if seen.insert(query.key()) {
                queries.push(query);
            }
        }
    }
    queries
}

fn submitted_pr_queries(urls: &[String]) -> Vec<PrNumberQuery> {
    let mut seen = HashSet::new();
    urls.iter()
        .filter_map(|url| PrNumberQuery::from_github_url(url))
        .filter(|query| seen.insert(query.key()))
        .collect()
}

pub(super) async fn fetch_pull_requests(
    sessions: &[Session],
    number_queries: &[PrNumberQuery],
) -> Result<HashMap<PullRequestKey, Option<PrProgressInfo>>, String> {
    let branch_queries = branch_pr_queries(sessions);
    if branch_queries.is_empty() && number_queries.is_empty() {
        return Ok(HashMap::new());
    }

    let client = tokio::task::spawn_blocking(GitHubClient::get)
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    let (branch_prs, number_prs) = tokio::try_join!(
        client.get_pr_progress_for_branches_batch(&branch_queries),
        client.get_pr_progress_for_numbers_batch(number_queries),
    )
    .map_err(|error| error.to_string())?;

    let mut prs = branch_prs
        .into_iter()
        .map(|(key, value)| (PullRequestKey::Branch(key), value))
        .collect::<HashMap<_, _>>();
    prs.extend(
        number_prs
            .into_iter()
            .map(|(key, value)| (PullRequestKey::Number(key), value)),
    );

    Ok(prs)
}
