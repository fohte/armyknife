use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::hash::Hash;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::EnrichmentUpdates;
use crate::commands::agent::tq_session_tasks::{
    SessionTask, build_task_by_session, fetch_session_tasks_with_parents,
};
use crate::commands::agent::types::Session;
use crate::infra::github::{BranchPrQuery, GitHubClient, PrInfo};
use crate::infra::tq::{TqClient, TqTaskSummary};

const CACHE_TTL: Duration = Duration::from_secs(30);
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
type BranchPrKey = (String, String, String);

#[derive(Clone)]
pub(super) struct SessionEnrichmentCache {
    tq: Arc<RefreshingCache<SessionTaskMetadata, String>>,
    pull_requests: Arc<RefreshingCache<HashMap<BranchPrKey, Option<PrInfo>>, BranchPrKey>>,
}

impl SessionEnrichmentCache {
    pub(super) fn new(updates: EnrichmentUpdates) -> Self {
        Self {
            tq: Arc::new(RefreshingCache::new(updates.clone(), FETCH_TIMEOUT)),
            pull_requests: Arc::new(RefreshingCache::new(updates, FETCH_TIMEOUT)),
        }
    }

    pub(super) async fn task_metadata(&self, sessions: &[Session]) -> SessionTaskMetadata {
        if sessions.is_empty() {
            return SessionTaskMetadata::default();
        }

        let session_ids = sessions
            .iter()
            .map(|session| session.session_id.clone())
            .collect::<HashSet<_>>();
        let fetch_session_ids = session_ids.clone();
        self.tq
            .get_or_refresh("tq", session_ids, move |previous| async move {
                fetch_task_metadata(fetch_session_ids, previous).await
            })
            .await
    }

    pub(super) async fn pull_requests(
        &self,
        sessions: &[Session],
    ) -> HashMap<String, Option<PrInfo>> {
        let query_keys = pull_request_keys(sessions);
        if query_keys.is_empty() {
            return HashMap::new();
        }

        let cached = self
            .pull_requests
            .get_or_refresh("GitHub", query_keys, {
                let sessions = sessions.to_vec();
                move |_| async move { fetch_pull_requests(&sessions).await }
            })
            .await;

        sessions
            .iter()
            .filter_map(|session| {
                let context = session.pr_context.as_ref()?;
                let key = BranchPrQuery {
                    owner: context.owner.clone(),
                    repo: context.repo.clone(),
                    branch: context.branch.clone(),
                }
                .key();
                Some((
                    session.session_id.clone(),
                    cached.get(&key).cloned().unwrap_or(None),
                ))
            })
            .collect()
    }
}

#[derive(Clone, Default)]
pub(super) struct SessionTaskMetadata {
    pub(super) links: HashMap<String, SessionTask>,
    pub(super) priorities: HashMap<String, Option<i32>>,
    pub(super) priority_order: HashMap<String, usize>,
}

struct RefreshingCache<T, K> {
    state: tokio::sync::Mutex<CacheState<T, K>>,
    #[cfg(test)]
    refresh_finished: tokio::sync::Notify,
    updates: EnrichmentUpdates,
    fetch_timeout: Duration,
}

struct CacheState<T, K> {
    value: Option<T>,
    last_attempt: Option<Instant>,
    attempted_keys: HashSet<K>,
    refresh_in_flight: bool,
}

impl<T, K> RefreshingCache<T, K>
where
    K: Eq + Hash,
{
    fn new(updates: EnrichmentUpdates, fetch_timeout: Duration) -> Self {
        Self {
            state: tokio::sync::Mutex::new(CacheState {
                value: None,
                last_attempt: None,
                attempted_keys: HashSet::new(),
                refresh_in_flight: false,
            }),
            #[cfg(test)]
            refresh_finished: tokio::sync::Notify::new(),
            updates,
            fetch_timeout,
        }
    }
}

impl<T, K> RefreshingCache<T, K>
where
    T: Clone + Default + Send + 'static,
    K: Clone + Eq + Hash + Send + 'static,
{
    #[cfg(test)]
    async fn current(&self) -> T {
        self.state.lock().await.value.clone().unwrap_or_default()
    }

    async fn get_or_refresh<F, Fut>(
        self: &Arc<Self>,
        resource: &'static str,
        keys: HashSet<K>,
        fetch: F,
    ) -> T
    where
        F: FnOnce(T) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, String>> + Send + 'static,
    {
        let mut state = self.state.lock().await;
        let cached = state.value.clone().unwrap_or_default();
        let expired = state
            .last_attempt
            .is_none_or(|last_attempt| last_attempt.elapsed() >= CACHE_TTL)
            || keys.iter().any(|key| !state.attempted_keys.contains(key));

        if expired && !state.refresh_in_flight {
            state.refresh_in_flight = true;
            state.attempted_keys.extend(keys);
            let cache = Arc::clone(self);
            let previous = cached.clone();
            let fetch_timeout = self.fetch_timeout;
            tokio::spawn(async move {
                let result = match tokio::time::timeout(fetch_timeout, fetch(previous)).await {
                    Ok(result) => result,
                    Err(_) => Err(format!("lookup exceeded {fetch_timeout:?}")),
                };
                let mut state = cache.state.lock().await;
                state.last_attempt = Some(Instant::now());
                match result {
                    Ok(value) => {
                        state.value = Some(value);
                    }
                    Err(error) => {
                        tracing::warn!(%error, resource, "Failed to refresh agent watch metadata; keeping cached values");
                    }
                };
                state.refresh_in_flight = false;
                drop(state);
                #[cfg(test)]
                cache.refresh_finished.notify_one();
                cache.updates.notify();
            });
        }

        cached
    }

    #[cfg(test)]
    async fn wait_until_idle(&self) {
        loop {
            let notified = self.refresh_finished.notified();
            if !self.state.lock().await.refresh_in_flight {
                return;
            }
            notified.await;
        }
    }
}

async fn fetch_task_metadata(
    session_ids: HashSet<String>,
    previous: SessionTaskMetadata,
) -> Result<SessionTaskMetadata, String> {
    let Some(client) = detected_tq_client().await? else {
        return Ok(previous);
    };
    let tq_sessions = fetch_session_tasks_with_parents(&client, &session_ids).await?;
    let links = build_task_by_session(tq_sessions, &session_ids);
    let task_ids = links
        .values()
        .map(|task| task.task_id.clone())
        .collect::<HashSet<_>>();
    if task_ids.is_empty() {
        return Ok(SessionTaskMetadata {
            links,
            priorities: previous.priorities,
            priority_order: previous.priority_order,
        });
    }

    let tasks = match client.list_tasks(&task_ids).await {
        Ok(tasks) => tasks,
        Err(error) => {
            tracing::warn!(%error, "Failed to fetch agent watch tq task priorities; keeping cached priorities");
            return Ok(SessionTaskMetadata {
                links,
                priorities: previous.priorities,
                priority_order: previous.priority_order,
            });
        }
    };

    let (priorities, priority_order) = task_priority_metadata(tasks);

    Ok(SessionTaskMetadata {
        links,
        priorities,
        priority_order,
    })
}

fn task_priority_metadata(
    tasks: Vec<TqTaskSummary>,
) -> (HashMap<String, Option<i32>>, HashMap<String, usize>) {
    let mut order_by_priority = HashMap::new();
    let mut priorities = HashMap::new();
    let mut priority_order = HashMap::new();
    for task in tasks {
        let next_order = order_by_priority.len();
        let order = *order_by_priority.entry(task.priority).or_insert(next_order);
        priorities.insert(task.id.clone(), task.priority);
        priority_order.insert(task.id, order);
    }

    (priorities, priority_order)
}

fn pull_request_keys(sessions: &[Session]) -> HashSet<BranchPrKey> {
    sessions
        .iter()
        .filter_map(|session| session.pr_context.as_ref())
        .map(|context| {
            BranchPrQuery {
                owner: context.owner.clone(),
                repo: context.repo.clone(),
                branch: context.branch.clone(),
            }
            .key()
        })
        .collect()
}

async fn detected_tq_client() -> Result<Option<TqClient>, String> {
    tokio::task::spawn_blocking(TqClient::detect)
        .await
        .map_err(|error| error.to_string())
}

async fn fetch_pull_requests(
    sessions: &[Session],
) -> Result<HashMap<BranchPrKey, Option<PrInfo>>, String> {
    let mut queries = Vec::new();
    let mut seen = HashSet::new();
    for session in sessions {
        let Some(context) = session.pr_context.as_ref() else {
            continue;
        };
        let query = BranchPrQuery {
            owner: context.owner.clone(),
            repo: context.repo.clone(),
            branch: context.branch.clone(),
        };
        if seen.insert(query.key()) {
            queries.push(query.clone());
        }
    }
    if queries.is_empty() {
        return Ok(HashMap::new());
    }

    let client = tokio::task::spawn_blocking(GitHubClient::get)
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    let prs = client
        .get_prs_for_branches_batch(&queries)
        .await
        .map_err(|error| error.to_string())?;

    Ok(prs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::{fixture, rstest};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Notify;

    #[fixture]
    fn refresh_cache() -> Arc<RefreshingCache<String, String>> {
        Arc::new(RefreshingCache {
            state: tokio::sync::Mutex::new(CacheState {
                value: Some("cached".to_string()),
                last_attempt: Some(Instant::now() - CACHE_TTL - Duration::from_secs(1)),
                attempted_keys: HashSet::from(["session-a".to_string()]),
                refresh_in_flight: false,
            }),
            refresh_finished: tokio::sync::Notify::new(),
            updates: EnrichmentUpdates::new(),
            fetch_timeout: FETCH_TIMEOUT,
        })
    }

    fn task(id: &str, priority: Option<i32>) -> TqTaskSummary {
        TqTaskSummary {
            id: id.to_string(),
            number: 1,
            title: "example task".to_string(),
            priority,
        }
    }

    #[rstest]
    #[case::unset_before_deferred(
        vec![task("task-a", None), task("task-b", Some(-1))],
        (
            HashMap::from([("task-a".to_string(), None), ("task-b".to_string(), Some(-1))]),
            HashMap::from([("task-a".to_string(), 0), ("task-b".to_string(), 1)]),
        ),
    )]
    #[case::same_priority_shares_order(
        vec![task("task-a", Some(0)), task("task-b", Some(0)), task("task-c", Some(-1))],
        (
            HashMap::from([
                ("task-a".to_string(), Some(0)),
                ("task-b".to_string(), Some(0)),
                ("task-c".to_string(), Some(-1)),
            ]),
            HashMap::from([
                ("task-a".to_string(), 0),
                ("task-b".to_string(), 0),
                ("task-c".to_string(), 1),
            ]),
        ),
    )]
    fn groups_tasks_in_tq_priority_order(
        #[case] tasks: Vec<TqTaskSummary>,
        #[case] expected: (HashMap<String, Option<i32>>, HashMap<String, usize>),
    ) {
        assert_eq!(task_priority_metadata(tasks), expected);
    }

    #[rstest]
    #[tokio::test]
    async fn refresh_is_single_flight_and_failure_keeps_the_latest_value(
        refresh_cache: Arc<RefreshingCache<String, String>>,
    ) {
        let cache = refresh_cache;
        let calls = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let first = {
            let calls = Arc::clone(&calls);
            let started = Arc::clone(&started);
            let release = Arc::clone(&release);
            cache
                .get_or_refresh(
                    "test",
                    HashSet::from(["session-a".to_string()]),
                    move |_| async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        started.notify_one();
                        release.notified().await;
                        Err("temporary failure".to_string())
                    },
                )
                .await
        };
        started.notified().await;
        let second = {
            let calls = Arc::clone(&calls);
            cache
                .get_or_refresh(
                    "test",
                    HashSet::from(["session-a".to_string()]),
                    move |_| async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok("unexpected".to_string())
                    },
                )
                .await
        };
        release.notify_one();
        cache.wait_until_idle().await;
        let after_failure = {
            let calls = Arc::clone(&calls);
            cache
                .get_or_refresh(
                    "test",
                    HashSet::from(["session-a".to_string()]),
                    move |_| async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok("unexpected".to_string())
                    },
                )
                .await
        };
        let cached = cache.current().await;

        assert_eq!(
            (
                first,
                second,
                after_failure,
                calls.load(Ordering::SeqCst),
                cached
            ),
            (
                "cached".to_string(),
                "cached".to_string(),
                "cached".to_string(),
                1,
                "cached".to_string()
            )
        );
    }

    #[rstest]
    #[tokio::test]
    async fn an_unattempted_key_refreshes_inside_the_ttl(
        refresh_cache: Arc<RefreshingCache<String, String>>,
    ) {
        let cache = refresh_cache;
        cache.state.lock().await.last_attempt = Some(Instant::now());
        let calls = Arc::new(AtomicUsize::new(0));
        let keys = HashSet::from(["session-a".to_string(), "session-b".to_string()]);
        let first = {
            let calls = Arc::clone(&calls);
            cache
                .get_or_refresh("test", keys.clone(), move |_| async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok("refreshed".to_string())
                })
                .await
        };
        cache.wait_until_idle().await;
        let second = {
            let calls = Arc::clone(&calls);
            cache
                .get_or_refresh("test", keys, move |_| async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok("unexpected".to_string())
                })
                .await
        };
        let refreshed = cache.current().await;

        assert_eq!(
            (first, second, calls.load(Ordering::SeqCst), refreshed),
            (
                "cached".to_string(),
                "refreshed".to_string(),
                1,
                "refreshed".to_string()
            )
        );
    }

    #[tokio::test]
    #[rstest]
    async fn successful_refresh_replaces_cached_value_and_notifies_clients(
        refresh_cache: Arc<RefreshingCache<String, String>>,
    ) {
        let cache = refresh_cache;
        let mut update_receiver = cache.updates.subscribe();

        let stale = cache
            .get_or_refresh(
                "test",
                HashSet::from(["session-a".to_string()]),
                |_| async { Ok("refreshed".to_string()) },
            )
            .await;
        tokio::time::timeout(Duration::from_secs(1), update_receiver.changed())
            .await
            .unwrap()
            .unwrap();
        let refreshed = cache.current().await;

        assert_eq!(
            (stale, refreshed),
            ("cached".to_string(), "refreshed".to_string())
        );
    }

    #[rstest]
    #[tokio::test]
    async fn timeout_releases_the_refresh_and_keeps_cached_value(
        refresh_cache: Arc<RefreshingCache<String, String>>,
    ) {
        let mut cache = refresh_cache;
        Arc::get_mut(&mut cache).unwrap().fetch_timeout = Duration::from_millis(10);
        let calls = Arc::new(AtomicUsize::new(0));
        let first = {
            let calls = Arc::clone(&calls);
            cache
                .get_or_refresh(
                    "test",
                    HashSet::from(["session-a".to_string()]),
                    move |_| async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        std::future::pending::<Result<String, String>>().await
                    },
                )
                .await
        };
        cache.wait_until_idle().await;
        let second = {
            let calls = Arc::clone(&calls);
            cache
                .get_or_refresh(
                    "test",
                    HashSet::from(["session-a".to_string()]),
                    move |_| async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok("unexpected".to_string())
                    },
                )
                .await
        };
        let state = cache.state.lock().await;

        assert_eq!(
            (
                first,
                second,
                state.value.clone().unwrap_or_default(),
                state.refresh_in_flight,
                calls.load(Ordering::SeqCst),
            ),
            (
                "cached".to_string(),
                "cached".to_string(),
                "cached".to_string(),
                false,
                1,
            ),
        );
    }
}
