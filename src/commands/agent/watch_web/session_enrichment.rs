use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::hash::Hash;
use std::pin::Pin;
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
type FetchFuture<T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'static>>;
type TaskMetadataFetcher = Arc<
    dyn Fn(HashSet<String>, SessionTaskMetadata) -> FetchFuture<SessionTaskMetadata> + Send + Sync,
>;
type PullRequestFetcher =
    Arc<dyn Fn(Vec<Session>) -> FetchFuture<HashMap<BranchPrKey, Option<PrInfo>>> + Send + Sync>;

#[derive(Clone)]
pub(super) struct SessionEnrichmentCache {
    tq: Arc<RefreshingCache<SessionTaskMetadata, String>>,
    pull_requests: Arc<RefreshingCache<HashMap<BranchPrKey, Option<PrInfo>>, BranchPrKey>>,
    task_fetcher: TaskMetadataFetcher,
    pull_request_fetcher: PullRequestFetcher,
}

impl SessionEnrichmentCache {
    pub(super) fn new(updates: EnrichmentUpdates) -> Self {
        let task_fetcher: TaskMetadataFetcher = Arc::new(
            |session_ids, previous| -> FetchFuture<SessionTaskMetadata> {
                Box::pin(fetch_task_metadata(session_ids, previous))
            },
        );
        let pull_request_fetcher: PullRequestFetcher = Arc::new(
            |sessions| -> FetchFuture<HashMap<BranchPrKey, Option<PrInfo>>> {
                Box::pin(async move { fetch_pull_requests(&sessions).await })
            },
        );
        Self::with_fetchers(updates, FETCH_TIMEOUT, task_fetcher, pull_request_fetcher)
    }

    fn with_fetchers(
        updates: EnrichmentUpdates,
        fetch_timeout: Duration,
        task_fetcher: TaskMetadataFetcher,
        pull_request_fetcher: PullRequestFetcher,
    ) -> Self {
        Self {
            tq: Arc::new(RefreshingCache::new(updates.clone(), fetch_timeout)),
            pull_requests: Arc::new(RefreshingCache::new(updates, fetch_timeout)),
            task_fetcher,
            pull_request_fetcher,
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
        let fetcher = Arc::clone(&self.task_fetcher);
        self.tq
            .get_or_refresh("tq", session_ids, move |previous| {
                fetcher(fetch_session_ids, previous)
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

        let fetcher = Arc::clone(&self.pull_request_fetcher);
        let cached = self
            .pull_requests
            .get_or_refresh("GitHub", query_keys, {
                let sessions = sessions.to_vec();
                move |_| fetcher(sessions)
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct SessionTaskMetadata {
    pub(super) links: HashMap<String, SessionTask>,
    pub(super) priorities: HashMap<String, Option<i32>>,
    pub(super) priority_order: HashMap<String, usize>,
}

struct RefreshingCache<T, K> {
    state: tokio::sync::Mutex<CacheState<T, K>>,
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
            state.attempted_keys = keys;
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
                cache.updates.notify();
            });
        }

        cached
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
    use crate::commands::agent::types::{Engine, Session, SessionPrContext, SessionStatus};
    use chrono::{TimeZone, Utc};
    use rstest::rstest;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Notify;

    fn cache_with_fetchers(
        updates: EnrichmentUpdates,
        fetch_timeout: Duration,
        task_fetcher: TaskMetadataFetcher,
        pull_request_fetcher: PullRequestFetcher,
    ) -> SessionEnrichmentCache {
        SessionEnrichmentCache::with_fetchers(
            updates,
            fetch_timeout,
            task_fetcher,
            pull_request_fetcher,
        )
    }

    fn task(id: &str, priority: Option<i32>) -> TqTaskSummary {
        TqTaskSummary {
            id: id.to_string(),
            number: 1,
            title: "example task".to_string(),
            priority,
        }
    }

    fn metadata(session_ids: HashSet<String>) -> SessionTaskMetadata {
        let links = session_ids
            .into_iter()
            .map(|session_id| {
                let task_id = format!("task-{session_id}");
                (
                    session_id,
                    SessionTask {
                        task_id,
                        task_number: 7,
                        task_title: "Example task".to_string(),
                        parent_task_id: None,
                        is_closed: false,
                    },
                )
            })
            .collect();
        SessionTaskMetadata {
            links,
            priorities: HashMap::new(),
            priority_order: HashMap::new(),
        }
    }

    fn session(session_id: &str, pr_context: Option<SessionPrContext>) -> Session {
        let timestamp = Utc.timestamp_opt(1_234_567, 0).single().unwrap();
        Session {
            session_id: session_id.to_string(),
            archived_at: None,
            pr_context,
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: PathBuf::from("/workspace/example"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Stopped,
            agent_status: None,
            agent_status_clear_pending: false,
            created_at: timestamp,
            updated_at: timestamp,
            last_message: None,
            current_tool: None,
            label: None,
            work_type: None,
            vibe_enabled: false,
            launched_with_vibe: false,
            work_type_pinned: false,
            child_launch: Default::default(),
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: Default::default(),
            pending_agent_task_ids: Default::default(),
            pending_permission_agent_ids: Default::default(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            engine: Engine::Claude,
            model: None,
            effort: None,
        }
    }

    fn session_with_branch(session_id: &str) -> Session {
        session(
            session_id,
            Some(SessionPrContext {
                owner: "example-owner".to_string(),
                repo: "example-repository".to_string(),
                branch: "feature/example".to_string(),
                checklist_item_id: None,
            }),
        )
    }

    async fn wait_for_refresh(receiver: &mut tokio::sync::watch::Receiver<()>) {
        tokio::time::timeout(Duration::from_secs(1), receiver.changed())
            .await
            .unwrap()
            .unwrap();
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

    #[tokio::test]
    async fn failed_session_refresh_is_single_flight_and_is_not_retried_for_the_same_keys() {
        let updates = EnrichmentUpdates::new();
        let mut receiver = updates.subscribe();
        let calls = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let fetch_calls = Arc::clone(&calls);
        let fetch_started = Arc::clone(&started);
        let fetch_release = Arc::clone(&release);
        let task_fetcher: TaskMetadataFetcher = Arc::new(move |session_ids, _| {
            let calls = Arc::clone(&fetch_calls);
            let started = Arc::clone(&fetch_started);
            let release = Arc::clone(&fetch_release);
            let call = calls.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                Box::pin(async move { Ok(metadata(session_ids)) })
            } else {
                Box::pin(async move {
                    started.notify_one();
                    release.notified().await;
                    Err("temporary failure".to_string())
                })
            }
        });
        let pull_request_fetcher: PullRequestFetcher =
            Arc::new(|_| Box::pin(async { Ok(HashMap::new()) }));
        let cache = cache_with_fetchers(updates, FETCH_TIMEOUT, task_fetcher, pull_request_fetcher);
        let session_a = session("session-a", None);

        let first = cache.task_metadata(std::slice::from_ref(&session_a)).await;
        wait_for_refresh(&mut receiver).await;
        let cached = cache.task_metadata(std::slice::from_ref(&session_a)).await;
        let session_b = session("session-b", None);
        let during_refresh = cache
            .task_metadata(&[session_a.clone(), session_b.clone()])
            .await;
        started.notified().await;
        let second_during_refresh = cache
            .task_metadata(&[session_a.clone(), session_b.clone()])
            .await;
        release.notify_one();
        wait_for_refresh(&mut receiver).await;
        let after_failure = cache.task_metadata(&[session_a, session_b]).await;

        assert_eq!(
            (
                first,
                cached,
                during_refresh,
                second_during_refresh,
                after_failure,
                calls.load(Ordering::SeqCst),
            ),
            (
                SessionTaskMetadata::default(),
                metadata(HashSet::from(["session-a".to_string()])),
                metadata(HashSet::from(["session-a".to_string()])),
                metadata(HashSet::from(["session-a".to_string()])),
                metadata(HashSet::from(["session-a".to_string()])),
                2,
            )
        );
    }

    #[tokio::test]
    async fn an_unattempted_session_refreshes_inside_the_ttl() {
        let updates = EnrichmentUpdates::new();
        let mut receiver = updates.subscribe();
        let calls = Arc::new(AtomicUsize::new(0));
        let fetch_calls = Arc::clone(&calls);
        let task_fetcher: TaskMetadataFetcher = Arc::new(move |session_ids, _| {
            let calls = Arc::clone(&fetch_calls);
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(metadata(session_ids))
            })
        });
        let pull_request_fetcher: PullRequestFetcher =
            Arc::new(|_| Box::pin(async { Ok(HashMap::new()) }));
        let cache = cache_with_fetchers(updates, FETCH_TIMEOUT, task_fetcher, pull_request_fetcher);
        let session_a = session("session-a", None);
        let session_b = session("session-b", None);

        let first = cache.task_metadata(std::slice::from_ref(&session_a)).await;
        wait_for_refresh(&mut receiver).await;
        let cached_a = cache.task_metadata(std::slice::from_ref(&session_a)).await;
        let stale_for_new_key = cache
            .task_metadata(&[session_a.clone(), session_b.clone()])
            .await;
        wait_for_refresh(&mut receiver).await;
        let refreshed = cache.task_metadata(&[session_a, session_b]).await;

        assert_eq!(
            (
                first,
                cached_a,
                stale_for_new_key,
                refreshed,
                calls.load(Ordering::SeqCst),
            ),
            (
                SessionTaskMetadata::default(),
                metadata(HashSet::from(["session-a".to_string()])),
                metadata(HashSet::from(["session-a".to_string()])),
                metadata(HashSet::from([
                    "session-a".to_string(),
                    "session-b".to_string(),
                ])),
                2,
            )
        );
    }

    #[tokio::test]
    async fn timeout_releases_refresh_and_retains_the_last_known_metadata() {
        let updates = EnrichmentUpdates::new();
        let mut receiver = updates.subscribe();
        let calls = Arc::new(AtomicUsize::new(0));
        let fetch_calls = Arc::clone(&calls);
        let task_fetcher: TaskMetadataFetcher = Arc::new(move |session_ids, _| {
            let calls = Arc::clone(&fetch_calls);
            let call = calls.fetch_add(1, Ordering::SeqCst);
            if call == 1 {
                Box::pin(async {
                    std::future::pending::<Result<SessionTaskMetadata, String>>().await
                })
            } else {
                Box::pin(async move { Ok(metadata(session_ids)) })
            }
        });
        let pull_request_fetcher: PullRequestFetcher =
            Arc::new(|_| Box::pin(async { Ok(HashMap::new()) }));
        let cache = cache_with_fetchers(
            updates,
            Duration::from_millis(10),
            task_fetcher,
            pull_request_fetcher,
        );
        let session_a = session("session-a", None);
        let session_b = session("session-b", None);
        let session_c = session("session-c", None);

        let first = cache.task_metadata(std::slice::from_ref(&session_a)).await;
        wait_for_refresh(&mut receiver).await;
        let primed = cache.task_metadata(std::slice::from_ref(&session_a)).await;
        let refresh_before_timeout = cache
            .task_metadata(&[session_a.clone(), session_b.clone()])
            .await;
        wait_for_refresh(&mut receiver).await;
        let after_timeout = cache
            .task_metadata(&[session_a.clone(), session_b.clone()])
            .await;
        let refresh_after_timeout = cache
            .task_metadata(&[session_a.clone(), session_b.clone(), session_c.clone()])
            .await;
        wait_for_refresh(&mut receiver).await;
        let refreshed = cache
            .task_metadata(&[session_a, session_b, session_c])
            .await;

        assert_eq!(
            (
                first,
                primed.clone(),
                refresh_before_timeout,
                after_timeout,
                refresh_after_timeout,
                refreshed,
                calls.load(Ordering::SeqCst),
            ),
            (
                SessionTaskMetadata::default(),
                metadata(HashSet::from(["session-a".to_string()])),
                metadata(HashSet::from(["session-a".to_string()])),
                metadata(HashSet::from(["session-a".to_string()])),
                metadata(HashSet::from(["session-a".to_string()])),
                metadata(HashSet::from([
                    "session-a".to_string(),
                    "session-b".to_string(),
                    "session-c".to_string(),
                ])),
                3,
            ),
        );
    }

    #[tokio::test]
    async fn branch_cached_pr_results_are_mapped_to_each_session() {
        let updates = EnrichmentUpdates::new();
        let mut receiver = updates.subscribe();
        let calls = Arc::new(AtomicUsize::new(0));
        let fetch_calls = Arc::clone(&calls);
        let pull_request_fetcher: PullRequestFetcher = Arc::new(move |_| {
            let calls = Arc::clone(&fetch_calls);
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(HashMap::from([(
                    (
                        "example-owner".to_string(),
                        "example-repository".to_string(),
                        "feature/example".to_string(),
                    ),
                    Some(PrInfo {
                        number: 17,
                        title: "Example change".to_string(),
                        state: crate::infra::github::PrState::Open,
                        url: "https://example.test/pull/17".to_string(),
                    }),
                )]))
            })
        });
        let task_fetcher: TaskMetadataFetcher =
            Arc::new(|_, previous| Box::pin(async move { Ok(previous) }));
        let cache = cache_with_fetchers(updates, FETCH_TIMEOUT, task_fetcher, pull_request_fetcher);
        let first = session_with_branch("session-one");
        let second = session_with_branch("session-two");

        let initial = cache.pull_requests(std::slice::from_ref(&first)).await;
        wait_for_refresh(&mut receiver).await;
        let shared_branch = cache.pull_requests(&[first.clone(), second.clone()]).await;

        assert_eq!(
            (initial, shared_branch, calls.load(Ordering::SeqCst)),
            (
                HashMap::from([("session-one".to_string(), None)]),
                HashMap::from([
                    (
                        "session-one".to_string(),
                        Some(PrInfo {
                            number: 17,
                            title: "Example change".to_string(),
                            state: crate::infra::github::PrState::Open,
                            url: "https://example.test/pull/17".to_string(),
                        }),
                    ),
                    (
                        "session-two".to_string(),
                        Some(PrInfo {
                            number: 17,
                            title: "Example change".to_string(),
                            state: crate::infra::github::PrState::Open,
                            url: "https://example.test/pull/17".to_string(),
                        }),
                    ),
                ]),
                1,
            ),
        );
    }
}
