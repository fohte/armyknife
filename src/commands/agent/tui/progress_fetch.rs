use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::hash::Hash;
use std::sync::{Arc, Mutex};

use serde::Serialize;

use crate::commands::agent::store;
use crate::commands::agent::types::{Session, SessionPrContext};
use crate::infra::git::GitRepo;
use crate::infra::github::{
    BranchPrQuery, GitHubClient, PrCiState, PrNumberQuery, PrProgressInfo, PrState,
};
use crate::infra::tq::{TqChecklist, TqClient, TqGithubLink};
use crate::shared::config::AgentConfig;

use super::super::session_pr_context::{self, Resolution};
use super::progress::{FetchedDelegate, FetchedProgress, ProgressFetchScope};
use super::progress_batches::{
    build_implementation_plan, pull_request_list_items, pull_request_queries,
};

type BranchPrKey = (String, String, String);
type PrNumberKey = (String, String, u64);
type ProgressResults<K> = HashMap<K, Option<PrProgressInfo>>;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum PullRequestCacheKey {
    Branch(BranchPrKey),
    Number(PrNumberKey),
}

trait PullRequestCacheQuery: Clone {
    type Key: Eq + Hash + Clone;

    fn key(&self) -> Self::Key;
    fn cache_key(key: &Self::Key) -> PullRequestCacheKey;
    fn related_cache_key(
        _key: &Self::Key,
        _progress: &PrProgressInfo,
    ) -> Option<PullRequestCacheKey> {
        None
    }
}

impl PullRequestCacheQuery for BranchPrQuery {
    type Key = BranchPrKey;

    fn key(&self) -> Self::Key {
        BranchPrQuery::key(self)
    }

    fn cache_key(key: &Self::Key) -> PullRequestCacheKey {
        PullRequestCacheKey::Branch(key.clone())
    }

    fn related_cache_key(
        key: &Self::Key,
        progress: &PrProgressInfo,
    ) -> Option<PullRequestCacheKey> {
        let (owner, repo, _) = key;
        Some(PullRequestCacheKey::Number((
            owner.clone(),
            repo.clone(),
            progress.pull_request.number,
        )))
    }
}

impl PullRequestCacheQuery for PrNumberQuery {
    type Key = PrNumberKey;

    fn key(&self) -> Self::Key {
        PrNumberQuery::key(self)
    }

    fn cache_key(key: &Self::Key) -> PullRequestCacheKey {
        PullRequestCacheKey::Number(key.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::commands::agent) struct PullRequestListItem {
    pub number: u64,
    pub owner: Option<String>,
    pub repo: Option<String>,
    pub title: String,
    pub url: String,
    pub head_sha: Option<String>,
    pub state: Option<PrState>,
    pub ci_state: Option<PrCiState>,
    pub review_status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::commands::agent) struct PullRequestListResponse {
    pub pull_requests: Vec<PullRequestListItem>,
    pub error: Option<String>,
}

#[derive(Clone, Default)]
pub(in crate::commands::agent) struct PullRequestListCache {
    progress: Arc<Mutex<HashMap<PullRequestCacheKey, PrProgressInfo>>>,
}

impl PullRequestListCache {
    fn queries_to_fetch<Q: PullRequestCacheQuery>(
        &self,
        queries: &[Q],
    ) -> (Vec<Q>, ProgressResults<Q::Key>) {
        let cached = self
            .progress
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut to_fetch = Vec::new();
        let mut results = HashMap::new();
        for query in queries {
            let key = query.key();
            if let Some(progress) = cached.get(&Q::cache_key(&key)) {
                results.insert(key, Some(progress.clone()));
            } else {
                to_fetch.push(query.clone());
            }
        }
        (to_fetch, results)
    }

    fn remember<Q: PullRequestCacheQuery>(&self, results: &ProgressResults<Q::Key>) {
        let mut cached = self
            .progress
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (key, progress) in results {
            let Some(progress) = progress
                .as_ref()
                .filter(|progress| is_terminal(&progress.pull_request.state))
            else {
                continue;
            };
            cached
                .entry(Q::cache_key(key))
                .or_insert_with(|| progress.clone());
            if let Some(related_key) = Q::related_cache_key(key, progress) {
                cached
                    .entry(related_key)
                    .or_insert_with(|| progress.clone());
            }
        }
    }
}

fn is_terminal(state: &PrState) -> bool {
    matches!(state, PrState::Merged | PrState::Closed)
}

pub(in crate::commands::agent) async fn fetch_pull_request_list(
    selected_session_id: String,
    task_id: Option<&str>,
    sessions: &[Session],
    task_ids_by_session: &HashMap<String, Vec<String>>,
    agent_config: &AgentConfig,
    cache: &PullRequestListCache,
) -> Result<PullRequestListResponse, String> {
    let request_scope =
        progress_scope(&selected_session_id, task_id, sessions, task_ids_by_session);
    let fetched = fetch_progress_for_sessions(
        selected_session_id,
        request_scope,
        sessions.to_vec(),
        Some(cache),
    )
    .await?;
    let pull_requests = pull_request_list_items(
        &fetched.delegated,
        fetched.implementation.as_ref(),
        agent_config,
    );
    Ok(PullRequestListResponse {
        pull_requests,
        error: fetched.github_error,
    })
}

async fn fetch_query_progress<Q, F, Fut>(
    cache: Option<&PullRequestListCache>,
    queries: &[Q],
    fetcher: F,
) -> (ProgressResults<Q::Key>, Option<String>)
where
    Q: PullRequestCacheQuery,
    F: FnOnce(Vec<Q>) -> Fut,
    Fut: Future<Output = Result<ProgressResults<Q::Key>, String>>,
{
    let (queries_to_fetch, mut results) = match cache {
        Some(cache) => cache.queries_to_fetch(queries),
        None => (queries.to_vec(), HashMap::new()),
    };
    if queries_to_fetch.is_empty() {
        return (results, None);
    }

    match fetcher(queries_to_fetch).await {
        Ok(fetched) => {
            if let Some(cache) = cache {
                cache.remember::<Q>(&fetched);
            }
            results.extend(fetched);
            (results, None)
        }
        Err(error) => (results, Some(error)),
    }
}

fn progress_scope(
    selected_session_id: &str,
    task_id: Option<&str>,
    sessions: &[Session],
    task_ids_by_session: &HashMap<String, Vec<String>>,
) -> ProgressFetchScope {
    let Some(task_id) = task_id else {
        return ProgressFetchScope {
            task_id: None,
            session_ids: None,
            other_task_session_ids: Vec::new(),
        };
    };

    let mut task_session_ids = task_ids_by_session
        .iter()
        .filter(|(session_id, _)| {
            sessions
                .iter()
                .any(|session| session.session_id.as_str() == session_id.as_str())
        })
        .filter(|(_, task_ids)| task_ids.iter().any(|id| id == task_id))
        .map(|(session_id, _)| session_id.clone())
        .collect::<HashSet<_>>();
    task_session_ids.insert(selected_session_id.to_string());
    let mut session_ids = task_session_ids.iter().cloned().collect::<Vec<_>>();
    session_ids.sort_unstable();
    let mut other_task_session_ids = sessions
        .iter()
        .filter(|session| !task_session_ids.contains(&session.session_id))
        .filter(|session| {
            task_ids_by_session
                .get(&session.session_id)
                .is_some_and(|task_ids| task_ids.iter().any(|id| id != task_id))
        })
        .map(|session| session.session_id.clone())
        .collect::<Vec<_>>();
    other_task_session_ids.sort_unstable();

    ProgressFetchScope {
        task_id: Some(task_id.to_string()),
        session_ids: Some(session_ids),
        other_task_session_ids,
    }
}

pub(super) async fn fetch_progress(
    selected_session_id: String,
    request_scope: ProgressFetchScope,
) -> Result<FetchedProgress, String> {
    let sessions = tokio::task::spawn_blocking(store::list_all_sessions)
        .await
        .map_err(|error| format!("failed to read agent sessions: {error}"))?
        .map_err(|error| format!("failed to read agent sessions: {error}"))?;
    fetch_progress_for_sessions(selected_session_id, request_scope, sessions, None).await
}

async fn fetch_progress_for_sessions(
    selected_session_id: String,
    request_scope: ProgressFetchScope,
    sessions: Vec<Session>,
    pull_request_cache: Option<&PullRequestListCache>,
) -> Result<FetchedProgress, String> {
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

    let query_data = tokio::task::spawn_blocking({
        let delegated = delegated.clone();
        move || branch_queries_for_sessions(&delegated)
    })
    .await
    .map_err(|error| format!("failed to inspect delegated worktrees: {error}"))?;

    let implementation_checklist =
        fetch_implementation_checklist(request_scope.task_id.as_deref()).await;
    let linked_pr_queries = implementation_checklist
        .as_ref()
        .map(|(checklist, github_links)| pull_request_queries(checklist, github_links))
        .unwrap_or_default();

    let (prs, mut github_error) = fetch_query_progress(
        pull_request_cache,
        &query_data.queries,
        |queries| async move {
            let client =
                GitHubClient::get().map_err(|error| format!("GitHub is unavailable: {error:#}"))?;
            client
                .get_pr_progress_for_branches_batch(&queries)
                .await
                .map_err(|error| format!("failed to fetch PR progress: {error:#}"))
        },
    )
    .await;
    let (linked_prs, linked_pr_error) = fetch_query_progress(
        pull_request_cache,
        &linked_pr_queries,
        |queries| async move {
            let client =
                GitHubClient::get().map_err(|error| format!("GitHub is unavailable: {error:#}"))?;
            client
                .get_pr_progress_for_numbers_batch(&queries)
                .await
                .map_err(|error| format!("failed to fetch linked PR progress: {error:#}"))
        },
    )
    .await;
    let linked_pr_lookup_failed = linked_pr_error.is_some();
    if let Some(error) = linked_pr_error {
        github_error.get_or_insert(error);
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
        request_scope: Some(request_scope),
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
    use std::cell::{Cell, RefCell};
    use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
    use std::path::PathBuf;

    use chrono::{TimeZone, Utc};
    use rstest::{fixture, rstest};

    use super::{
        BranchLookup, ProgressFetchScope, PullRequestListCache, Resolution, branch_query_for_with,
        direct_children, fetch_query_progress, progress_scope, task_descendant_children,
    };
    use crate::commands::agent::types::{Engine, Session, SessionPrContext, SessionStatus};
    use crate::infra::github::{
        BranchPrQuery, PrCiState, PrInfo, PrNumberQuery, PrProgressInfo, PrState,
    };

    #[tokio::test]
    async fn pull_request_list_fetch_skips_terminal_branches_but_refreshes_open_and_missing() {
        let cache = PullRequestListCache::default();
        let terminal = BranchPrQuery {
            owner: "sample-owner".to_string(),
            repo: "sample-repo".to_string(),
            branch: "finished-branch".to_string(),
        };
        let open = BranchPrQuery {
            branch: "active-branch".to_string(),
            ..terminal.clone()
        };
        let missing = BranchPrQuery {
            branch: "unlinked-branch".to_string(),
            ..terminal.clone()
        };
        let terminal_progress = progress(PrState::Merged, 2048);
        let queries = vec![terminal.clone(), open.clone(), missing.clone()];
        let fetched_queries = RefCell::new(Vec::new());
        let (first_results, first_error) =
            fetch_query_progress(Some(&cache), &queries, |requested| {
                fetched_queries.borrow_mut().push(requested);
                let results = HashMap::from([
                    (terminal.key(), Some(terminal_progress.clone())),
                    (open.key(), Some(progress(PrState::Open, 2046))),
                    (missing.key(), None),
                ]);
                async move { Ok(results) }
            })
            .await;

        let terminal_number = PrNumberQuery {
            owner: terminal.owner.clone(),
            repo: terminal.repo.clone(),
            number: 2048,
        };
        let number_fetch_called = Cell::new(false);
        let (number_results, number_error) =
            fetch_query_progress(Some(&cache), std::slice::from_ref(&terminal_number), |_| {
                number_fetch_called.set(true);
                async { Ok(HashMap::new()) }
            })
            .await;

        let (second_results, second_error) =
            fetch_query_progress(Some(&cache), &queries, |requested| {
                fetched_queries.borrow_mut().push(requested.clone());
                let results = requested
                    .iter()
                    .map(|query| {
                        let progress = (query == &open).then(|| progress(PrState::Open, 2046));
                        (query.key(), progress)
                    })
                    .collect::<HashMap<_, _>>();
                async move { Ok(results) }
            })
            .await;

        assert_eq!(
            (
                fetched_queries.into_inner(),
                first_results,
                first_error,
                number_results,
                number_fetch_called.get(),
                number_error,
                second_results,
                second_error,
            ),
            (
                vec![queries.clone(), vec![open.clone(), missing.clone()]],
                HashMap::from([
                    (terminal.key(), Some(terminal_progress.clone())),
                    (open.key(), Some(progress(PrState::Open, 2046))),
                    (missing.key(), None),
                ]),
                None,
                HashMap::from([(terminal_number.key(), Some(terminal_progress.clone()))]),
                false,
                None,
                HashMap::from([
                    (terminal.key(), Some(terminal_progress)),
                    (open.key(), Some(progress(PrState::Open, 2046))),
                    (missing.key(), None),
                ]),
                None,
            ),
        );
    }

    #[tokio::test]
    async fn pull_request_list_fetch_skips_cached_closed_number_queries() {
        let cache = PullRequestListCache::default();
        let closed = PrNumberQuery {
            owner: "sample-owner".to_string(),
            repo: "sample-repo".to_string(),
            number: 2048,
        };
        let open = PrNumberQuery {
            number: 2046,
            ..closed.clone()
        };
        let queries = vec![closed.clone(), open.clone()];
        let fetched_queries = RefCell::new(Vec::new());
        let (first_results, first_error) =
            fetch_query_progress(Some(&cache), &queries, |requested| {
                fetched_queries.borrow_mut().push(requested);
                let results = HashMap::from([
                    (closed.key(), Some(progress(PrState::Closed, 2048))),
                    (open.key(), Some(progress(PrState::Open, 2046))),
                ]);
                async move { Ok(results) }
            })
            .await;
        let (second_results, second_error) =
            fetch_query_progress(Some(&cache), &queries, |requested| {
                fetched_queries.borrow_mut().push(requested.clone());
                let results = requested
                    .iter()
                    .map(|query| (query.key(), Some(progress(PrState::Open, 2046))))
                    .collect::<HashMap<_, _>>();
                async move { Ok(results) }
            })
            .await;

        assert_eq!(
            (
                fetched_queries.into_inner(),
                first_results,
                first_error,
                second_results,
                second_error,
            ),
            (
                vec![queries, vec![open.clone()]],
                HashMap::from([
                    (closed.key(), Some(progress(PrState::Closed, 2048))),
                    (open.key(), Some(progress(PrState::Open, 2046))),
                ]),
                None,
                HashMap::from([
                    (closed.key(), Some(progress(PrState::Closed, 2048))),
                    (open.key(), Some(progress(PrState::Open, 2046))),
                ]),
                None,
            ),
        );
    }

    #[tokio::test]
    async fn pull_request_list_fetch_keeps_a_newer_terminal_result_when_an_older_lookup_finishes_late()
     {
        let cache = PullRequestListCache::default();
        let query = BranchPrQuery {
            owner: "sample-owner".to_string(),
            repo: "sample-repo".to_string(),
            branch: "sample-branch".to_string(),
        };
        let (older_tx, older_rx) = tokio::sync::oneshot::channel();
        let (older_started_tx, older_started_rx) = tokio::sync::oneshot::channel();
        let older_cache = cache.clone();
        let older_query = query.clone();
        let older_request = tokio::spawn(async move {
            fetch_query_progress(
                Some(&older_cache),
                std::slice::from_ref(&older_query),
                move |_| async move {
                    older_started_tx.send(()).unwrap();
                    Ok(older_rx.await.unwrap())
                },
            )
            .await
        });
        older_started_rx.await.unwrap();

        let (newer_tx, newer_rx) = tokio::sync::oneshot::channel();
        let (newer_started_tx, newer_started_rx) = tokio::sync::oneshot::channel();
        let newer_cache = cache.clone();
        let newer_query = query.clone();
        let newer_request = tokio::spawn(async move {
            fetch_query_progress(
                Some(&newer_cache),
                std::slice::from_ref(&newer_query),
                move |_| async move {
                    newer_started_tx.send(()).unwrap();
                    Ok(newer_rx.await.unwrap())
                },
            )
            .await
        });
        newer_started_rx.await.unwrap();

        let newer_progress = progress(PrState::Merged, 2048);
        newer_tx
            .send(HashMap::from([(query.key(), Some(newer_progress.clone()))]))
            .unwrap();
        let newer_result = newer_request.await.unwrap();

        let older_progress = progress(PrState::Closed, 2048);
        older_tx
            .send(HashMap::from([(query.key(), Some(older_progress.clone()))]))
            .unwrap();
        let older_result = older_request.await.unwrap();

        let fetch_called = Cell::new(false);
        let (cached_result, cache_error) =
            fetch_query_progress(Some(&cache), std::slice::from_ref(&query), |_| {
                fetch_called.set(true);
                async { Ok(HashMap::new()) }
            })
            .await;

        assert_eq!(
            (
                newer_result,
                older_result,
                cached_result,
                cache_error,
                fetch_called.get(),
            ),
            (
                (HashMap::from([(query.key(), Some(newer_progress))]), None),
                (HashMap::from([(query.key(), Some(older_progress))]), None),
                HashMap::from([(query.key(), Some(progress(PrState::Merged, 2048)))]),
                None,
                false,
            ),
        );
    }

    fn progress(state: PrState, number: u64) -> PrProgressInfo {
        PrProgressInfo {
            pull_request: PrInfo {
                number,
                title: "A sample change".to_string(),
                state,
                url: format!("https://github.com/sample-owner/sample-repo/pull/{number}"),
            },
            ci_state: PrCiState::Successful,
            head_sha: None,
        }
    }

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
    fn task_scope_includes_sessions_linked_to_the_primary_task_and_other_task_barriers() {
        let sessions = vec![
            session("owner", vec![], 1),
            session("same-task", vec!["owner"], 2),
            session("other-task", vec!["owner"], 3),
            session("unlinked", vec!["owner"], 4),
        ];
        let task_ids_by_session = HashMap::from([
            ("owner".to_string(), vec!["task-main".to_string()]),
            ("same-task".to_string(), vec!["task-main".to_string()]),
            ("other-task".to_string(), vec!["task-other".to_string()]),
        ]);

        assert_eq!(
            progress_scope("owner", Some("task-main"), &sessions, &task_ids_by_session),
            ProgressFetchScope {
                task_id: Some("task-main".to_string()),
                session_ids: Some(vec!["owner".to_string(), "same-task".to_string()]),
                other_task_session_ids: vec!["other-task".to_string()],
            },
        );
    }

    #[test]
    fn taskless_scope_uses_direct_children() {
        let sessions = vec![session("owner", vec![], 1)];

        assert_eq!(
            progress_scope("owner", None, &sessions, &HashMap::new()),
            ProgressFetchScope {
                task_id: None,
                session_ids: None,
                other_task_session_ids: Vec::new(),
            },
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
            web_uis: Vec::new(),
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
            current_tool_name: None,
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
