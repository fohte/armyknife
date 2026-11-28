use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::hash::Hash;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use super::EnrichmentUpdates;
use crate::commands::agent::tq_session_tasks::{
    SessionTask, build_all_tasks_by_session, fetch_session_tasks_with_parents,
    order_tasks_by_priority, task_priority_metadata,
};
use crate::commands::agent::types::Session;
use crate::infra::github::{PrNumberQuery, PrProgressInfo, PrState};
use crate::infra::tq::{
    TqChangeEvent, TqChangeResource, TqClient, TqEvent, TqGithubLink, TqTaskSummary,
};
use tokio::time::Instant;

mod cache;
mod task_projects;
mod task_pull_requests;
#[cfg(test)]
mod test_support;

use cache::{merge_session_task_metadata, prune_session_task_metadata};
use task_projects::fetch_task_projects;

const CACHE_TTL: Duration = Duration::from_secs(30);
const CACHE_KEY_RETENTION: Duration = Duration::from_secs(10 * 60);
const FETCH_TIMEOUT: Duration = Duration::from_secs(35);
type BranchPrKey = (String, String, String);
type NumberPrKey = (String, String, u64);
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum PullRequestKey {
    Branch(BranchPrKey),
    Number(NumberPrKey),
}
type FetchFuture<T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'static>>;
type TaskMetadataFetcher = Arc<
    dyn Fn(HashSet<String>, SessionTaskMetadata) -> FetchFuture<SessionTaskMetadata> + Send + Sync,
>;
type PullRequestFetcher = Arc<
    dyn Fn(
            Vec<Session>,
            Vec<PrNumberQuery>,
        ) -> FetchFuture<HashMap<PullRequestKey, Option<PrProgressInfo>>>
        + Send
        + Sync,
>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SessionPullRequestInfo {
    pub(super) owner: String,
    pub(super) repo: String,
    pub(super) progress: PrProgressInfo,
    pub(super) from_cwd_branch: bool,
}

#[derive(Clone)]
pub(super) struct SessionEnrichmentCache {
    tq: Arc<RefreshingCache<SessionTaskMetadata, String>>,
    pull_requests:
        Arc<RefreshingCache<HashMap<PullRequestKey, Option<PrProgressInfo>>, PullRequestKey>>,
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
            |sessions,
             number_queries|
             -> FetchFuture<HashMap<PullRequestKey, Option<PrProgressInfo>>> {
                Box::pin(async move {
                    task_pull_requests::fetch_pull_requests(&sessions, &number_queries).await
                })
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
            tq: Arc::new(RefreshingCache::new(
                updates.clone(),
                fetch_timeout,
                CACHE_TTL,
            )),
            pull_requests: Arc::new(RefreshingCache::new(
                updates.clone(),
                fetch_timeout,
                CACHE_TTL,
            )),
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
            .flat_map(|session| {
                std::iter::once(session.session_id.clone())
                    .chain(session.ancestor_session_ids.iter().cloned())
            })
            .collect::<HashSet<_>>();
        let fetch_session_ids = session_ids.clone();
        let fetcher = Arc::clone(&self.task_fetcher);
        self.tq
            .get_or_refresh(
                "tq",
                session_ids,
                None,
                move |previous, _| fetcher(fetch_session_ids, previous),
                CacheOperations {
                    merge: merge_session_task_metadata,
                    prune: prune_session_task_metadata,
                    can_refresh: |_, _| true,
                    fetched_keys: |requested, _| requested.clone(),
                },
            )
            .await
    }

    pub(super) async fn apply_tq_event(&self, event: &TqEvent) {
        match event {
            TqEvent::Connected => {
                self.tq.invalidate(true).await;
                self.tq.updates.notify();
            }
            TqEvent::Change(TqChangeEvent {
                resource:
                    TqChangeResource::Task
                    | TqChangeResource::Project
                    | TqChangeResource::AgentSession
                    | TqChangeResource::Github
                    | TqChangeResource::Unknown,
                ..
            }) => {
                self.tq.invalidate(false).await;
                self.tq.updates.notify();
            }
            TqEvent::Change(_) => {}
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct SessionTaskMetadata {
    pub(super) links: HashMap<String, Vec<SessionTask>>,
    pub(super) updated_at: HashMap<String, chrono::DateTime<chrono::Utc>>,
    pub(super) priorities: HashMap<String, Option<i32>>,
    pub(super) priority_order: HashMap<String, usize>,
    pub(super) task_projects: HashMap<String, String>,
    pub(super) project_names: HashMap<String, String>,
    pub(super) github_links: HashMap<String, Vec<TqGithubLink>>,
}

struct RefreshingCache<T, K> {
    state: tokio::sync::Mutex<CacheState<T, K>>,
    updates: EnrichmentUpdates,
    fetch_timeout: Duration,
}

struct CacheOperations<T, K> {
    merge: fn(&mut T, T, &HashSet<K>),
    prune: fn(&mut T, &HashSet<K>),
    can_refresh: fn(&T, &K) -> bool,
    fetched_keys: fn(&HashSet<K>, &HashSet<K>) -> HashSet<K>,
}

struct CacheState<T, K> {
    value: T,
    attempted_at: HashMap<K, Instant>,
    last_requested: HashMap<K, Instant>,
    refresh_in_flight: bool,
    cache_ttl: Option<Duration>,
    generation: u64,
}

impl<T, K> RefreshingCache<T, K>
where
    T: Default,
    K: Eq + Hash,
{
    fn new(updates: EnrichmentUpdates, fetch_timeout: Duration, cache_ttl: Duration) -> Self {
        Self {
            state: tokio::sync::Mutex::new(CacheState {
                value: T::default(),
                attempted_at: HashMap::new(),
                last_requested: HashMap::new(),
                refresh_in_flight: false,
                cache_ttl: Some(cache_ttl),
                generation: 0,
            }),
            updates,
            fetch_timeout,
        }
    }

    async fn invalidate(&self, stop_ttl: bool) {
        let mut state = self.state.lock().await;
        state.attempted_at.clear();
        state.generation = state.generation.wrapping_add(1);
        if stop_ttl {
            state.cache_ttl = None;
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
        fetch_timeout: Option<Duration>,
        fetch: F,
        operations: CacheOperations<T, K>,
    ) -> T
    where
        F: FnOnce(T, HashSet<K>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, String>> + Send + 'static,
    {
        let mut state = self.state.lock().await;
        let now = Instant::now();
        for key in &keys {
            state.last_requested.insert(key.clone(), now);
        }
        let stale_keys = state
            .last_requested
            .iter()
            .filter(|(_, last_requested)| last_requested.elapsed() >= CACHE_KEY_RETENTION)
            .map(|(key, _)| key.clone())
            .collect::<HashSet<_>>();
        for key in &stale_keys {
            state.last_requested.remove(key);
            state.attempted_at.remove(key);
        }
        (operations.prune)(&mut state.value, &stale_keys);
        let cached = state.value.clone();
        let expired_keys = keys
            .iter()
            .filter(|key| {
                (operations.can_refresh)(&state.value, key)
                    && state.attempted_at.get(*key).is_none_or(|last_attempt| {
                        state
                            .cache_ttl
                            .is_some_and(|ttl| last_attempt.elapsed() >= ttl)
                    })
            })
            .cloned()
            .collect::<HashSet<_>>();

        if !expired_keys.is_empty() && !state.refresh_in_flight {
            state.refresh_in_flight = true;
            let generation = state.generation;
            let cache = Arc::clone(self);
            let previous = cached.clone();
            let fetched_keys = (operations.fetched_keys)(&keys, &expired_keys);
            let expired_keys = expired_keys.clone();
            let fetch_timeout = fetch_timeout.unwrap_or(self.fetch_timeout);
            tokio::spawn(async move {
                let result = match tokio::time::timeout(
                    fetch_timeout,
                    fetch(previous, expired_keys),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => Err(format!("lookup exceeded {fetch_timeout:?}")),
                };
                let mut state = cache.state.lock().await;
                if state.generation == generation {
                    let attempted_at = Instant::now();
                    for key in &fetched_keys {
                        state.attempted_at.insert(key.clone(), attempted_at);
                    }
                }
                match result {
                    Ok(value) => {
                        (operations.merge)(&mut state.value, value, &fetched_keys);
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
    let mut links = build_all_tasks_by_session(tq_sessions, &session_ids);
    let task_ids = links
        .values()
        .flatten()
        .map(|task| task.task_id.clone())
        .collect::<HashSet<_>>();
    if task_ids.is_empty() {
        return Ok(SessionTaskMetadata {
            links,
            updated_at: previous.updated_at,
            priorities: previous.priorities,
            priority_order: previous.priority_order,
            task_projects: previous.task_projects,
            project_names: previous.project_names,
            github_links: previous.github_links,
        });
    }

    let tasks = match client.list_tasks(&task_ids).await {
        Ok(tasks) => tasks,
        Err(error) => {
            tracing::warn!(%error, "Failed to fetch agent watch tq task metadata including GitHub links; keeping cached values");
            return Ok(keep_cached_priority_metadata(links, previous));
        }
    };

    let updated_at = task_update_timestamps(&tasks);
    let github_links = task_github_links(&tasks);

    let (priorities, priority_order) = task_priority_metadata(tasks.clone());
    order_tasks_by_priority(&mut links, &priority_order);
    let (task_projects, project_names) = fetch_task_projects(
        tasks,
        previous.project_names,
        previous.task_projects,
        || async {
            client
                .list_projects()
                .await
                .map_err(|error| error.to_string())
        },
    )
    .await;

    Ok(SessionTaskMetadata {
        links,
        updated_at,
        priorities,
        priority_order,
        task_projects,
        project_names,
        github_links,
    })
}

fn task_github_links(tasks: &[TqTaskSummary]) -> HashMap<String, Vec<TqGithubLink>> {
    tasks
        .iter()
        .map(|task| (task.id.clone(), task.github_links.clone()))
        .collect()
}

fn task_update_timestamps(
    tasks: &[TqTaskSummary],
) -> HashMap<String, chrono::DateTime<chrono::Utc>> {
    tasks
        .iter()
        .filter_map(|task| {
            task.updated_at
                .map(|updated_at| (task.id.clone(), updated_at))
        })
        .collect()
}

fn keep_cached_priority_metadata(
    mut links: HashMap<String, Vec<SessionTask>>,
    previous: SessionTaskMetadata,
) -> SessionTaskMetadata {
    order_tasks_by_priority(&mut links, &previous.priority_order);
    SessionTaskMetadata { links, ..previous }
}

fn pull_request_can_refresh(
    cached: &HashMap<PullRequestKey, Option<PrProgressInfo>>,
    key: &PullRequestKey,
) -> bool {
    // Completed PRs stay cached so they do not trigger recurring lookups.
    !matches!(
        cached
            .get(key)
            .and_then(Option::as_ref)
            .map(|pr| &pr.pull_request.state),
        Some(PrState::Merged | PrState::Closed)
    )
}

async fn detected_tq_client() -> Result<Option<TqClient>, String> {
    tokio::task::spawn_blocking(TqClient::detect)
        .await
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::test_support::{
        branch_key, cache_with_fetchers, metadata, session, session_with_branch,
        session_with_branch_name, wait_for_refresh,
    };
    use super::*;
    use crate::commands::agent::tq_session_tasks::task_priority_metadata;
    use crate::infra::github::{PrCiState, PrInfo, PrProgressInfo, PrState};
    use crate::infra::tq::TqGithubLink;
    use rstest::rstest;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Notify;

    fn tq_change(resource: TqChangeResource) -> TqEvent {
        TqEvent::Change(TqChangeEvent {
            resource,
            id: None,
            origin: None,
            task_ids: None,
        })
    }

    fn head_info(number: u64) -> PrProgressInfo {
        PrProgressInfo {
            pull_request: PrInfo {
                number,
                title: "Example change".to_string(),
                state: PrState::Open,
                url: format!("https://example.test/pull/{number}"),
            },
            ci_state: PrCiState::Successful,
            head_sha: Some("sample-head".to_string()),
        }
    }

    fn task(id: &str, priority: Option<i32>) -> TqTaskSummary {
        TqTaskSummary {
            id: id.to_string(),
            number: 1,
            title: "example task".to_string(),
            updated_at: None,
            priority,
            project_id: None,
            github_links: Vec::new(),
        }
    }

    fn linked_task(id: &str) -> SessionTask {
        SessionTask {
            task_id: id.to_string(),
            task_number: 1,
            task_title: id.to_string(),
            parent_task_id: None,
            is_closed: false,
        }
    }

    #[rstest]
    #[case::unset_priority_keeps_its_tq_position(
        vec![task("task-a", Some(0)), task("task-b", None), task("task-c", Some(-1))],
        (
            HashMap::from([
                ("task-a".to_string(), Some(0)),
                ("task-b".to_string(), None),
                ("task-c".to_string(), Some(-1)),
            ]),
            HashMap::from([
                ("task-a".to_string(), 0),
                ("task-b".to_string(), 1),
                ("task-c".to_string(), 2),
            ]),
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

    #[test]
    fn cached_priority_metadata_orders_new_links_after_task_lookup_failure() {
        let previous = SessionTaskMetadata {
            links: HashMap::new(),
            updated_at: HashMap::new(),
            priorities: HashMap::from([
                ("higher".to_string(), Some(0)),
                ("lower".to_string(), Some(1)),
            ]),
            priority_order: HashMap::from([("higher".to_string(), 0), ("lower".to_string(), 1)]),
            task_projects: HashMap::from([("higher".to_string(), "sample project".to_string())]),
            project_names: HashMap::new(),
            github_links: HashMap::from([(
                "higher".to_string(),
                vec![tq_pr_link("linked-pr", 29)],
            )]),
        };
        let links = HashMap::from([(
            "session-a".to_string(),
            vec![linked_task("lower"), linked_task("higher")],
        )]);

        assert_eq!(
            keep_cached_priority_metadata(links, previous.clone()),
            SessionTaskMetadata {
                links: HashMap::from([(
                    "session-a".to_string(),
                    vec![linked_task("higher"), linked_task("lower")],
                )]),
                ..previous
            },
        );
    }

    #[test]
    fn maps_tq_task_timestamps_by_task_id() {
        let updated_at = chrono::DateTime::parse_from_rfc3339("2026-10-02T03:04:05Z")
            .expect("valid timestamp")
            .with_timezone(&chrono::Utc);
        let mut updated_task = task("task-a", None);
        updated_task.updated_at = Some(updated_at);

        assert_eq!(
            task_update_timestamps(&[updated_task, task("task-b", None)]),
            HashMap::from([("task-a".to_string(), updated_at)]),
        );
    }

    #[tokio::test]
    async fn refresh_includes_ancestor_session_task_projects() {
        let updates = EnrichmentUpdates::new();
        let mut receiver = updates.subscribe();
        let task_fetcher: TaskMetadataFetcher = Arc::new(|session_ids, _| {
            Box::pin(async move {
                let mut result = metadata(session_ids.clone());
                result.task_projects = session_ids
                    .iter()
                    .map(|session_id| {
                        (
                            format!("task-{session_id}"),
                            format!("project-{session_id}"),
                        )
                    })
                    .collect();
                Ok(result)
            })
        });
        let pull_request_fetcher: PullRequestFetcher =
            Arc::new(|_, _| Box::pin(async { Ok(HashMap::new()) }));
        let cache = cache_with_fetchers(updates, FETCH_TIMEOUT, task_fetcher, pull_request_fetcher);
        let mut child = session("child-session", None);
        child.ancestor_session_ids = vec!["parent-session".to_string()];

        let initial = cache.task_metadata(std::slice::from_ref(&child)).await;
        wait_for_refresh(&mut receiver).await;
        let refreshed = cache.task_metadata(std::slice::from_ref(&child)).await;
        let expected_ids =
            HashSet::from(["child-session".to_string(), "parent-session".to_string()]);
        let mut expected = metadata(expected_ids);
        expected.task_projects = HashMap::from([
            (
                "task-child-session".to_string(),
                "project-child-session".to_string(),
            ),
            (
                "task-parent-session".to_string(),
                "project-parent-session".to_string(),
            ),
        ]);

        assert_eq!(
            (initial, refreshed),
            (SessionTaskMetadata::default(), expected)
        );
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
            Arc::new(|_, _| Box::pin(async { Ok(HashMap::new()) }));
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
            Arc::new(|_, _| Box::pin(async { Ok(HashMap::new()) }));
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

    #[tokio::test(start_paused = true)]
    async fn refresh_timestamps_cover_every_session_fetched_in_the_batch() {
        let updates = EnrichmentUpdates::new();
        let mut receiver = updates.subscribe();
        let fetches = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded_fetches = Arc::clone(&fetches);
        let task_fetcher: TaskMetadataFetcher = Arc::new(move |session_ids, _| {
            let recorded_fetches = Arc::clone(&recorded_fetches);
            Box::pin(async move {
                recorded_fetches.lock().unwrap().push(session_ids.clone());
                Ok(metadata(session_ids))
            })
        });
        let pull_request_fetcher: PullRequestFetcher =
            Arc::new(|_, _| Box::pin(async { Ok(HashMap::new()) }));
        let cache = cache_with_fetchers(updates, FETCH_TIMEOUT, task_fetcher, pull_request_fetcher);
        let session_a = session("session-a", None);
        let session_b = session("session-b", None);

        let _ = cache.task_metadata(std::slice::from_ref(&session_a)).await;
        wait_for_refresh(&mut receiver).await;
        tokio::time::advance(Duration::from_secs(20)).await;
        let _ = cache
            .task_metadata(&[session_a.clone(), session_b.clone()])
            .await;
        wait_for_refresh(&mut receiver).await;
        tokio::time::advance(Duration::from_secs(11)).await;
        let _ = cache.task_metadata(&[session_a, session_b]).await;

        assert_eq!(
            fetches.lock().unwrap().clone(),
            vec![
                HashSet::from(["session-a".to_string()]),
                HashSet::from(["session-a".to_string(), "session-b".to_string()]),
            ]
        );
    }

    #[rstest]
    #[case::connected(TqEvent::Connected, true)]
    #[case::task(tq_change(TqChangeResource::Task), true)]
    #[case::project(tq_change(TqChangeResource::Project), true)]
    #[case::agent_session(tq_change(TqChangeResource::AgentSession), true)]
    #[case::github(tq_change(TqChangeResource::Github), true)]
    #[case::unknown(tq_change(TqChangeResource::Unknown), true)]
    #[case::checklist_item(tq_change(TqChangeResource::ChecklistItem), false)]
    #[tokio::test]
    async fn tq_events_invalidate_session_task_metadata(
        #[case] event: TqEvent,
        #[case] should_invalidate: bool,
    ) {
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
            Arc::new(|_, _| Box::pin(async { Ok(HashMap::new()) }));
        let cache = cache_with_fetchers(updates, FETCH_TIMEOUT, task_fetcher, pull_request_fetcher);
        let session = session("session-a", None);

        let _ = cache.task_metadata(std::slice::from_ref(&session)).await;
        wait_for_refresh(&mut receiver).await;
        cache.apply_tq_event(&event).await;
        let notified = receiver.has_changed().is_ok_and(|changed| changed);
        if should_invalidate {
            let _ = receiver.changed().await;
            let _ = cache.task_metadata(std::slice::from_ref(&session)).await;
            wait_for_refresh(&mut receiver).await;
        }

        assert_eq!(
            (notified, calls.load(Ordering::SeqCst)),
            (should_invalidate, if should_invalidate { 2 } else { 1 }),
        );
    }

    #[tokio::test(start_paused = true)]
    async fn connected_stops_the_tq_cache_ttl_after_existing_ttl_expiry() {
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
            Arc::new(|_, _| Box::pin(async { Ok(HashMap::new()) }));
        let cache = cache_with_fetchers(updates, FETCH_TIMEOUT, task_fetcher, pull_request_fetcher);
        let session = session("session-a", None);

        let _ = cache.task_metadata(std::slice::from_ref(&session)).await;
        wait_for_refresh(&mut receiver).await;
        tokio::time::advance(CACHE_TTL).await;
        let _ = cache.task_metadata(std::slice::from_ref(&session)).await;
        wait_for_refresh(&mut receiver).await;
        cache.apply_tq_event(&TqEvent::Connected).await;
        let _ = receiver.changed().await;
        tokio::time::advance(CACHE_TTL * 2).await;
        let _ = cache.task_metadata(std::slice::from_ref(&session)).await;

        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn invalidation_during_fetch_causes_another_refresh() {
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
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                started.notify_one();
                release.notified().await;
                Ok(metadata(session_ids))
            })
        });
        let pull_request_fetcher: PullRequestFetcher =
            Arc::new(|_, _| Box::pin(async { Ok(HashMap::new()) }));
        let cache = cache_with_fetchers(updates, FETCH_TIMEOUT, task_fetcher, pull_request_fetcher);
        let session = session("session-a", None);

        let _ = cache.task_metadata(std::slice::from_ref(&session)).await;
        started.notified().await;
        cache
            .apply_tq_event(&tq_change(TqChangeResource::Task))
            .await;
        let _ = receiver.changed().await;
        release.notify_one();
        wait_for_refresh(&mut receiver).await;
        let _ = cache.task_metadata(std::slice::from_ref(&session)).await;
        started.notified().await;
        release.notify_one();
        wait_for_refresh(&mut receiver).await;

        assert_eq!(calls.load(Ordering::SeqCst), 2);
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
            Arc::new(|_, _| Box::pin(async { Ok(HashMap::new()) }));
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
        let pull_request_fetcher: PullRequestFetcher = Arc::new(move |_, _| {
            let calls = Arc::clone(&fetch_calls);
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(HashMap::from([(
                    PullRequestKey::Branch((
                        "example-owner".to_string(),
                        "example-repository".to_string(),
                        "feature/example".to_string(),
                    )),
                    Some(head_info(17)),
                )]))
            })
        });
        let task_fetcher: TaskMetadataFetcher =
            Arc::new(|_, previous| Box::pin(async move { Ok(previous) }));
        let cache = cache_with_fetchers(updates, FETCH_TIMEOUT, task_fetcher, pull_request_fetcher);
        let first = session_with_branch("session-one");
        let second = session_with_branch("session-two");

        let initial = cache
            .pull_requests(
                std::slice::from_ref(&first),
                &SessionTaskMetadata::default(),
            )
            .await;
        wait_for_refresh(&mut receiver).await;
        let shared_branch = cache
            .pull_requests(
                &[first.clone(), second.clone()],
                &SessionTaskMetadata::default(),
            )
            .await;

        let first_pr = SessionPullRequestInfo {
            owner: "example-owner".to_string(),
            repo: "example-repository".to_string(),
            progress: head_info(17),
            from_cwd_branch: true,
        };
        let second_pr = SessionPullRequestInfo {
            owner: "example-owner".to_string(),
            repo: "example-repository".to_string(),
            progress: head_info(17),
            from_cwd_branch: true,
        };
        assert_eq!(
            (initial, shared_branch, calls.load(Ordering::SeqCst)),
            (
                HashMap::from([("session-one".to_string(), Vec::new())]),
                HashMap::from([
                    ("session-one".to_string(), vec![first_pr]),
                    ("session-two".to_string(), vec![second_pr]),
                ]),
                1,
            ),
        );
    }

    #[tokio::test]
    async fn task_linked_prs_join_existing_prs_without_duplicates() {
        let updates = EnrichmentUpdates::new();
        let mut receiver = updates.subscribe();
        let requested_number_queries = Arc::new(std::sync::Mutex::new(Vec::new()));
        let requested_queries = Arc::clone(&requested_number_queries);
        let pull_request_fetcher: PullRequestFetcher = Arc::new(move |_, number_queries| {
            let requested_queries = Arc::clone(&requested_queries);
            Box::pin(async move {
                *requested_queries
                    .lock()
                    .expect("query recorder lock should not be poisoned") = number_queries.clone();
                let mut results = number_queries
                    .into_iter()
                    .map(|query| {
                        (
                            PullRequestKey::Number(query.key()),
                            Some(head_info(query.number)),
                        )
                    })
                    .collect::<HashMap<_, _>>();
                results.insert(branch_key("cwd-branch"), Some(head_info(17)));
                Ok(results)
            })
        });
        let task_fetcher: TaskMetadataFetcher =
            Arc::new(|_, previous| Box::pin(async move { Ok(previous) }));
        let cache = cache_with_fetchers(updates, FETCH_TIMEOUT, task_fetcher, pull_request_fetcher);
        let mut current_session = session_with_branch_name("session-multiple", "cwd-branch");
        let cwd_context = current_session
            .pr_context
            .clone()
            .expect("cwd context exists");
        current_session.created_pr_urls = vec![
            "https://github.com/example-owner/example-repository/pull/23".to_string(),
            "https://github.com/example-owner/example-repository/pull/23".to_string(),
        ];
        let mut linked_cwd_pr = tq_pr_link("linked-cwd-pr", 17);
        linked_cwd_pr.owner = Some("EXAMPLE-OWNER".to_string());
        linked_cwd_pr.repo = Some("EXAMPLE-REPOSITORY".to_string());
        linked_cwd_pr.url =
            Some("https://github.com/EXAMPLE-OWNER/EXAMPLE-REPOSITORY/pull/17".to_string());
        let mut task_only_pr = tq_pr_link("linked-task-pr", 29);
        task_only_pr.url = None;
        let mut list_task = task("task-current", None);
        list_task.github_links = vec![
            linked_cwd_pr,
            tq_pr_link("linked-created-pr", 23),
            task_only_pr,
        ];
        let task_metadata = SessionTaskMetadata {
            links: HashMap::from([(
                current_session.session_id.clone(),
                vec![linked_task("task-current")],
            )]),
            github_links: task_github_links(&[list_task]),
            ..SessionTaskMetadata::default()
        };

        let initial = cache
            .pull_requests(std::slice::from_ref(&current_session), &task_metadata)
            .await;
        wait_for_refresh(&mut receiver).await;
        let pull_requests = cache
            .pull_requests(std::slice::from_ref(&current_session), &task_metadata)
            .await;

        assert_eq!(
            (
                initial,
                pull_requests,
                requested_number_queries
                    .lock()
                    .expect("query recorder lock should not be poisoned")
                    .clone()
            ),
            (
                HashMap::from([("session-multiple".to_string(), Vec::new())]),
                HashMap::from([(
                    "session-multiple".to_string(),
                    vec![
                        SessionPullRequestInfo {
                            owner: cwd_context.owner,
                            repo: cwd_context.repo,
                            progress: head_info(17),
                            from_cwd_branch: true,
                        },
                        SessionPullRequestInfo {
                            owner: "example-owner".to_string(),
                            repo: "example-repository".to_string(),
                            progress: head_info(23),
                            from_cwd_branch: false,
                        },
                        SessionPullRequestInfo {
                            owner: "example-owner".to_string(),
                            repo: "example-repository".to_string(),
                            progress: head_info(29),
                            from_cwd_branch: false,
                        },
                    ],
                )]),
                vec![
                    pr_query(23),
                    pr_query_with_owner_repo("EXAMPLE-OWNER", "EXAMPLE-REPOSITORY", 17),
                    pr_query(29),
                ],
            ),
        );
    }

    fn tq_pr_link(id: &str, number: u64) -> TqGithubLink {
        TqGithubLink {
            id: id.to_string(),
            kind: Some("pull_request".to_string()),
            owner: Some("example-owner".to_string()),
            repo: Some("example-repository".to_string()),
            number: Some(number),
            title: None,
            url: Some(format!(
                "https://github.com/example-owner/example-repository/pull/{number}"
            )),
            state: None,
        }
    }

    fn pr_query(number: u64) -> PrNumberQuery {
        pr_query_with_owner_repo("example-owner", "example-repository", number)
    }

    fn pr_query_with_owner_repo(owner: &str, repo: &str, number: u64) -> PrNumberQuery {
        PrNumberQuery {
            owner: owner.to_string(),
            repo: repo.to_string(),
            number,
        }
    }
}
