use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::time::Instant;

use futures_util::{StreamExt, stream};

use crate::commands::agent::types::{Session, SessionPrContext};
use crate::infra::github::{BranchPrQuery, GitHubClient, PrNumberQuery, PrProgressInfo};
use crate::infra::tq::TqGithubLink;

use super::{
    PullRequestKey, SessionEnrichmentCache, SessionPullRequestInfo, SessionTaskMetadata,
    TASK_GITHUB_LINK_CACHE_TTL,
    cache::{merge_pull_request_results, prune_pull_request_results},
};

const TASK_GITHUB_LINK_FETCH_CONCURRENCY: usize = 4;

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
                {
                    let sessions = sessions.to_vec();
                    move |_| fetcher(sessions, number_queries)
                },
                merge_pull_request_results,
                prune_pull_request_results,
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

pub(super) async fn fetch_task_github_links<F, Fut>(
    task_ids: &HashSet<String>,
    previous: &HashMap<String, Vec<TqGithubLink>>,
    previous_fetched_at: &HashMap<String, Instant>,
    fetch: F,
) -> (HashMap<String, Vec<TqGithubLink>>, HashMap<String, Instant>)
where
    F: Fn(String) -> Fut + Sync,
    Fut: Future<Output = Result<Vec<TqGithubLink>, String>> + Send,
{
    let now = Instant::now();
    let expired_task_ids = task_ids
        .iter()
        .filter(|task_id| {
            previous_fetched_at
                .get(*task_id)
                .is_none_or(|fetched_at| fetched_at.elapsed() >= TASK_GITHUB_LINK_CACHE_TTL)
        })
        .cloned()
        .collect::<HashSet<_>>();
    let fetch = &fetch;
    let responses = stream::iter(expired_task_ids.iter().cloned().map(|task_id| async move {
        let result = fetch(task_id.clone()).await;
        (task_id, result)
    }))
    .buffer_unordered(TASK_GITHUB_LINK_FETCH_CONCURRENCY)
    .collect::<Vec<_>>()
    .await;

    let mut links = task_ids
        .iter()
        .filter(|task_id| !expired_task_ids.contains(*task_id))
        .filter_map(|task_id| {
            previous
                .get(task_id)
                .map(|links| (task_id.clone(), links.clone()))
        })
        .collect::<HashMap<_, _>>();
    let mut fetched_at = task_ids
        .iter()
        .filter(|task_id| !expired_task_ids.contains(*task_id))
        .filter_map(|task_id| {
            previous_fetched_at
                .get(task_id)
                .map(|fetched_at| (task_id.clone(), *fetched_at))
        })
        .collect::<HashMap<_, _>>();

    for (task_id, result) in responses {
        fetched_at.insert(task_id.clone(), now);
        match result {
            Ok(task_links) => {
                links.insert(task_id, task_links);
            }
            Err(error) => {
                tracing::warn!(
                    event = "agent.watch_web.task_github_links.fetch_failed",
                    %error,
                    task_id,
                    "Failed to fetch agent watch tq task GitHub links; keeping cached values"
                );
                if let Some(cached_links) = previous.get(&task_id) {
                    links.insert(task_id, cached_links.clone());
                }
            }
        }
    }

    (links, fetched_at)
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use crate::infra::tq::TqGithubLink;

    use super::{TASK_GITHUB_LINK_FETCH_CONCURRENCY, fetch_task_github_links};

    fn github_link(id: &str) -> TqGithubLink {
        TqGithubLink {
            id: id.to_string(),
            kind: Some("pull_request".to_string()),
            owner: Some("example-owner".to_string()),
            repo: Some("example-repository".to_string()),
            number: Some(29),
            title: None,
            url: None,
            state: None,
        }
    }

    #[tokio::test]
    async fn does_not_fetch_task_github_links_within_cache_ttl() {
        let task_ids = HashSet::from(["task-fresh".to_string()]);
        let previous = HashMap::from([("task-fresh".to_string(), vec![github_link("cached")])]);
        let previous_fetched_at = HashMap::from([("task-fresh".to_string(), Instant::now())]);
        let requested = Arc::new(Mutex::new(Vec::new()));
        let fetch_requested = Arc::clone(&requested);

        let (links, fetched_at) =
            fetch_task_github_links(&task_ids, &previous, &previous_fetched_at, move |task_id| {
                fetch_requested.lock().unwrap().push(task_id);
                async { Ok(Vec::new()) }
            })
            .await;

        assert_eq!(
            (links, fetched_at, requested.lock().unwrap().clone()),
            (previous, previous_fetched_at, Vec::<String>::new()),
        );
    }

    #[tokio::test]
    async fn refreshes_expired_task_links_with_bounded_concurrency() {
        let task_ids = (0..12)
            .map(|index| format!("task-{index}"))
            .collect::<HashSet<_>>();
        let previous_fetched_at = task_ids
            .iter()
            .map(|task_id| {
                (
                    task_id.clone(),
                    Instant::now() - Duration::from_secs(6 * 60),
                )
            })
            .collect::<HashMap<_, _>>();
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let requested = Arc::new(Mutex::new(Vec::new()));

        let (links, _) =
            fetch_task_github_links(&task_ids, &HashMap::new(), &previous_fetched_at, {
                let active = Arc::clone(&active);
                let max_active = Arc::clone(&max_active);
                let requested = Arc::clone(&requested);
                move |task_id| {
                    requested.lock().unwrap().push(task_id);
                    let active = Arc::clone(&active);
                    let max_active = Arc::clone(&max_active);
                    async move {
                        let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                        max_active.fetch_max(current, Ordering::SeqCst);
                        tokio::task::yield_now().await;
                        active.fetch_sub(1, Ordering::SeqCst);
                        Ok(Vec::new())
                    }
                }
            })
            .await;

        let expected_links = task_ids
            .iter()
            .map(|task_id| (task_id.clone(), Vec::new()))
            .collect::<HashMap<String, Vec<TqGithubLink>>>();
        let mut requested = requested.lock().unwrap().clone();
        requested.sort();
        let mut expected_requested = task_ids.iter().cloned().collect::<Vec<_>>();
        expected_requested.sort();
        assert_eq!(
            (links, requested, max_active.load(Ordering::SeqCst)),
            (
                expected_links,
                expected_requested,
                TASK_GITHUB_LINK_FETCH_CONCURRENCY,
            ),
        );
    }

    #[tokio::test]
    async fn keeps_cached_task_github_links_when_a_refresh_fails() {
        let task_ids = HashSet::from(["task-example".to_string()]);
        let previous = HashMap::from([(
            "task-example".to_string(),
            vec![TqGithubLink {
                id: "link-example".to_string(),
                kind: Some("pull_request".to_string()),
                owner: Some("example-owner".to_string()),
                repo: Some("example-repository".to_string()),
                number: Some(29),
                title: None,
                url: None,
                state: None,
            }],
        )]);

        let (links, fetched_at) =
            fetch_task_github_links(&task_ids, &previous, &HashMap::new(), |_| async {
                Err("temporary lookup failure".to_string())
            })
            .await;

        assert_eq!(
            (links, fetched_at.keys().cloned().collect::<HashSet<_>>(),),
            (previous, HashSet::from(["task-example".to_string()]),),
        );
    }
}
