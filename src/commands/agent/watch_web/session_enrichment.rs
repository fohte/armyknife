use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::hash::Hash;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::EnrichmentUpdates;
use crate::commands::agent::tq_session_tasks::{
    SessionTask, build_all_tasks_by_session, fetch_session_tasks_with_parents,
    order_tasks_by_priority, task_priority_metadata,
};
use crate::commands::agent::types::Session;
use crate::infra::github::{PrNumberQuery, PrProgressInfo};
use crate::infra::tq::{
    TQ_COMMAND_TIMEOUT, TqClient, TqGithubLink, TqProjectSummary, TqTaskSummary,
};

mod cache;
mod task_pull_requests;
#[cfg(test)]
mod test_support;

use cache::{merge_session_task_metadata, prune_session_task_metadata};

const CACHE_TTL: Duration = Duration::from_secs(30);
const CACHE_KEY_RETENTION: Duration = Duration::from_secs(10 * 60);
const FETCH_TIMEOUT: Duration = Duration::from_secs(35);
const TASK_GITHUB_LINK_CACHE_TTL: Duration = Duration::from_secs(5 * 60);
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
type TaskGithubLinkFetcher = Arc<
    dyn Fn(
            HashSet<String>,
            HashMap<String, Vec<TqGithubLink>>,
        ) -> FetchFuture<HashMap<String, Vec<TqGithubLink>>>
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
    task_github_links: Arc<RefreshingCache<HashMap<String, Vec<TqGithubLink>>, String>>,
    task_fetcher: TaskMetadataFetcher,
    pull_request_fetcher: PullRequestFetcher,
    task_github_link_fetcher: TaskGithubLinkFetcher,
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
        let task_github_link_fetcher: TaskGithubLinkFetcher = Arc::new(
            |task_ids, previous| -> FetchFuture<HashMap<String, Vec<TqGithubLink>>> {
                Box::pin(async move {
                    Ok(task_pull_requests::fetch_task_github_links(
                        &task_ids,
                        &previous,
                        |task_id| async move {
                            TqClient
                                .task_github_links(&task_id)
                                .await
                                .map_err(|error| error.to_string())
                        },
                    )
                    .await)
                })
            },
        );
        Self::with_fetchers(
            updates,
            FETCH_TIMEOUT,
            task_fetcher,
            pull_request_fetcher,
            task_github_link_fetcher,
        )
    }

    fn with_fetchers(
        updates: EnrichmentUpdates,
        fetch_timeout: Duration,
        task_fetcher: TaskMetadataFetcher,
        pull_request_fetcher: PullRequestFetcher,
        task_github_link_fetcher: TaskGithubLinkFetcher,
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
            task_github_links: Arc::new(RefreshingCache::new(
                updates,
                FETCH_TIMEOUT,
                TASK_GITHUB_LINK_CACHE_TTL,
            )),
            task_fetcher,
            pull_request_fetcher,
            task_github_link_fetcher,
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
        let mut metadata = self
            .tq
            .get_or_refresh(
                "tq",
                session_ids,
                None,
                move |previous, _| fetcher(fetch_session_ids, previous),
                merge_session_task_metadata,
                prune_session_task_metadata,
            )
            .await;

        let task_ids = metadata
            .links
            .values()
            .flatten()
            .map(|task| task.task_id.clone())
            .collect::<HashSet<_>>();
        if !task_ids.is_empty() {
            let fetch_timeout = task_github_link_fetch_timeout(task_ids.len());
            let fetcher = Arc::clone(&self.task_github_link_fetcher);
            metadata.github_links = self
                .task_github_links
                .get_or_refresh(
                    "TQ task GitHub links",
                    task_ids,
                    Some(fetch_timeout),
                    move |previous, expired_task_ids| fetcher(expired_task_ids, previous),
                    cache::merge_task_github_links,
                    cache::prune_task_github_links,
                )
                .await;
        } else {
            metadata.github_links.clear();
        }

        metadata
    }
}

fn task_github_link_fetch_timeout(task_count: usize) -> Duration {
    let fetch_waves = task_count.div_ceil(task_pull_requests::TASK_GITHUB_LINK_FETCH_CONCURRENCY);
    let fetch_waves = u32::try_from(fetch_waves).unwrap_or(u32::MAX);
    TQ_COMMAND_TIMEOUT
        .saturating_mul(fetch_waves)
        .saturating_add(FETCH_TIMEOUT)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct SessionTaskMetadata {
    pub(super) links: HashMap<String, Vec<SessionTask>>,
    pub(super) updated_at: HashMap<String, chrono::DateTime<chrono::Utc>>,
    pub(super) priorities: HashMap<String, Option<i32>>,
    pub(super) priority_order: HashMap<String, usize>,
    pub(super) task_projects: HashMap<String, String>,
    pub(super) github_links: HashMap<String, Vec<TqGithubLink>>,
}

struct RefreshingCache<T, K> {
    state: tokio::sync::Mutex<CacheState<T, K>>,
    updates: EnrichmentUpdates,
    fetch_timeout: Duration,
    cache_ttl: Duration,
}

struct CacheState<T, K> {
    value: T,
    attempted_at: HashMap<K, Instant>,
    last_requested: HashMap<K, Instant>,
    refresh_in_flight: bool,
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
            }),
            updates,
            fetch_timeout,
            cache_ttl,
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
        merge: fn(&mut T, T, &HashSet<K>),
        prune: fn(&mut T, &HashSet<K>),
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
        prune(&mut state.value, &stale_keys);
        let cached = state.value.clone();
        let expired_keys = keys
            .iter()
            .filter(|key| {
                state
                    .attempted_at
                    .get(*key)
                    .is_none_or(|last_attempt| last_attempt.elapsed() >= self.cache_ttl)
            })
            .cloned()
            .collect::<HashSet<_>>();

        if !expired_keys.is_empty() && !state.refresh_in_flight {
            state.refresh_in_flight = true;
            let cache = Arc::clone(self);
            let previous = cached.clone();
            let attempted_keys = expired_keys.clone();
            let fetch_timeout = fetch_timeout.unwrap_or(self.fetch_timeout);
            tokio::spawn(async move {
                let result = match tokio::time::timeout(
                    fetch_timeout,
                    fetch(previous, attempted_keys.clone()),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => Err(format!("lookup exceeded {fetch_timeout:?}")),
                };
                let mut state = cache.state.lock().await;
                let attempted_at = Instant::now();
                for key in &attempted_keys {
                    state.attempted_at.insert(key.clone(), attempted_at);
                }
                match result {
                    Ok(value) => {
                        merge(&mut state.value, value, &keys);
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
            github_links: previous.github_links,
        });
    }

    let tasks = match client.list_tasks(&task_ids).await {
        Ok(tasks) => tasks,
        Err(error) => {
            tracing::warn!(%error, "Failed to fetch agent watch tq task priorities; keeping cached priorities");
            return Ok(keep_cached_priority_metadata(links, previous));
        }
    };

    let updated_at = task_update_timestamps(&tasks);

    let (priorities, priority_order) = task_priority_metadata(tasks.clone());
    order_tasks_by_priority(&mut links, &priority_order);
    let task_projects = fetch_task_projects(tasks, previous.task_projects, || async {
        client
            .list_projects()
            .await
            .map_err(|error| error.to_string())
    })
    .await;

    Ok(SessionTaskMetadata {
        links,
        updated_at,
        priorities,
        priority_order,
        task_projects,
        github_links: previous.github_links,
    })
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

async fn fetch_task_projects<F, Fut>(
    tasks: Vec<TqTaskSummary>,
    previous: HashMap<String, String>,
    fetch_projects: F,
) -> HashMap<String, String>
where
    F: FnOnce() -> Fut + Send,
    Fut: Future<Output = Result<Vec<TqProjectSummary>, String>> + Send,
{
    if !tasks.iter().any(|task| task.project_id.is_some()) {
        return HashMap::new();
    }

    match fetch_projects().await {
        Ok(projects) => task_project_names(tasks, projects),
        Err(error) => {
            tracing::warn!(%error, "Failed to fetch agent watch tq projects; keeping cached values");
            previous
        }
    }
}

fn task_project_names(
    tasks: Vec<TqTaskSummary>,
    projects: Vec<TqProjectSummary>,
) -> HashMap<String, String> {
    let project_names = projects
        .into_iter()
        .map(|project| (project.id, project.title))
        .collect::<HashMap<_, _>>();

    tasks
        .into_iter()
        .filter_map(|task| {
            let project_name = project_names.get(task.project_id.as_deref()?)?;
            Some((task.id, project_name.clone()))
        })
        .collect()
}

async fn detected_tq_client() -> Result<Option<TqClient>, String> {
    tokio::task::spawn_blocking(TqClient::detect)
        .await
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::test_support::{
        branch_key, cache_with_fetchers, cache_with_task_github_link_fetcher, metadata, session,
        session_with_branch, session_with_branch_name, wait_for_refresh,
    };
    use super::*;
    use crate::commands::agent::tq_session_tasks::task_priority_metadata;
    use crate::infra::github::{PrCiState, PrInfo, PrProgressInfo, PrState};
    use crate::infra::tq::TqGithubLink;
    use rstest::rstest;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Notify;

    type TaskGithubLinkMap = HashMap<String, Vec<TqGithubLink>>;
    type TaskGithubLinkCache = Arc<RefreshingCache<TaskGithubLinkMap, String>>;

    fn task_link_fetcher(requested: Arc<Mutex<Vec<HashSet<String>>>>) -> TaskGithubLinkFetcher {
        Arc::new(move |task_ids, _| {
            requested.lock().unwrap().push(task_ids.clone());
            Box::pin(async move {
                Ok(task_ids
                    .into_iter()
                    .map(|task_id| (task_id, Vec::new()))
                    .collect())
            })
        })
    }

    async fn refresh_task_links(
        cache: &TaskGithubLinkCache,
        task_ids: HashSet<String>,
        requested: Arc<Mutex<Vec<HashSet<String>>>>,
    ) -> TaskGithubLinkMap {
        let fetcher = task_link_fetcher(requested);
        cache
            .get_or_refresh(
                "TQ task GitHub links",
                task_ids,
                None,
                move |previous, expired_task_ids| fetcher(expired_task_ids, previous),
                cache::merge_task_github_links,
                cache::prune_task_github_links,
            )
            .await
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
            github_links: HashMap::new(),
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
    fn maps_tq_task_projects_to_project_titles() {
        let mut task = task("task-a", None);
        task.project_id = Some("project-a".to_string());
        let tasks = vec![task];
        let projects = vec![TqProjectSummary {
            id: "project-a".to_string(),
            title: "Example project".to_string(),
        }];

        assert_eq!(
            task_project_names(tasks, projects),
            HashMap::from([("task-a".to_string(), "Example project".to_string())]),
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

    #[rstest]
    #[case::project_list_succeeds(
        Ok(vec![TqProjectSummary {
            id: "project-a".to_string(),
            title: "Sample project".to_string(),
        }]),
        HashMap::new(),
        HashMap::from([("task-a".to_string(), "Sample project".to_string())]),
    )]
    #[case::project_list_failure_keeps_cached_values(
        Err("temporary failure".to_string()),
        HashMap::from([("task-a".to_string(), "Cached project".to_string())]),
        HashMap::from([("task-a".to_string(), "Cached project".to_string())]),
    )]
    #[tokio::test]
    async fn fetches_project_names_or_keeps_cached_values_on_error(
        #[case] project_result: std::result::Result<Vec<TqProjectSummary>, String>,
        #[case] previous: HashMap<String, String>,
        #[case] expected: HashMap<String, String>,
    ) {
        let mut linked_task = task("task-a", None);
        linked_task.project_id = Some("project-a".to_string());

        let actual = fetch_task_projects(vec![linked_task], previous, move || async move {
            project_result
        })
        .await;

        assert_eq!(actual, expected);
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
    async fn slow_task_github_lookup_does_not_delay_task_metadata() {
        let updates = EnrichmentUpdates::new();
        let mut receiver = updates.subscribe();
        let task_fetcher: TaskMetadataFetcher =
            Arc::new(|session_ids, _| Box::pin(async move { Ok(metadata(session_ids)) }));
        let pull_request_fetcher: PullRequestFetcher =
            Arc::new(|_, _| Box::pin(async { Ok(HashMap::new()) }));
        let calls = Arc::new(AtomicUsize::new(0));
        let fetch_calls = Arc::clone(&calls);
        let fetch_started = Arc::new(Notify::new());
        let fetch_started_signal = Arc::clone(&fetch_started);
        let release_fetch = Arc::new(Notify::new());
        let release_fetch_signal = Arc::clone(&release_fetch);
        let task_github_link_fetcher: TaskGithubLinkFetcher = Arc::new(move |_, previous| {
            let calls = Arc::clone(&fetch_calls);
            let fetch_started = Arc::clone(&fetch_started_signal);
            let release_fetch = Arc::clone(&release_fetch_signal);
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                fetch_started.notify_one();
                release_fetch.notified().await;
                Ok(previous)
            })
        });
        let cache = cache_with_task_github_link_fetcher(
            updates,
            Duration::from_secs(1),
            task_fetcher,
            pull_request_fetcher,
            task_github_link_fetcher,
        );
        let session = session("session-a", None);

        let _ = cache.task_metadata(std::slice::from_ref(&session)).await;
        wait_for_refresh(&mut receiver).await;
        let refreshed = tokio::time::timeout(
            Duration::from_millis(20),
            cache.task_metadata(std::slice::from_ref(&session)),
        )
        .await
        .ok();
        fetch_started.notified().await;
        release_fetch.notify_one();
        wait_for_refresh(&mut receiver).await;
        let after_link_refresh = cache.task_metadata(std::slice::from_ref(&session)).await;

        assert_eq!(
            (refreshed, after_link_refresh, calls.load(Ordering::SeqCst)),
            (
                Some(metadata(HashSet::from(["session-a".to_string()]))),
                metadata(HashSet::from(["session-a".to_string()])),
                1,
            ),
        );
    }

    #[tokio::test]
    async fn task_github_link_cache_refreshes_only_expired_tasks() {
        let updates = EnrichmentUpdates::new();
        let mut receiver = updates.subscribe();
        let requested = Arc::new(Mutex::new(Vec::new()));
        let cache_ttl = Duration::from_secs(2);
        let cache = Arc::new(RefreshingCache::new(updates, FETCH_TIMEOUT, cache_ttl));
        let task_a = "task-a".to_string();
        let task_b = "task-b".to_string();
        let keys_a = HashSet::from([task_a.clone()]);
        let keys_a_b = HashSet::from([task_a.clone(), task_b.clone()]);

        let initial = refresh_task_links(&cache, keys_a.clone(), Arc::clone(&requested)).await;
        receiver
            .changed()
            .await
            .expect("initial task-link refresh should notify");
        let reused_within_ttl =
            refresh_task_links(&cache, keys_a.clone(), Arc::clone(&requested)).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let before_task_b_refresh =
            refresh_task_links(&cache, keys_a_b.clone(), Arc::clone(&requested)).await;
        receiver
            .changed()
            .await
            .expect("new task-link refresh should notify");
        let after_task_b_refresh =
            refresh_task_links(&cache, keys_a_b.clone(), Arc::clone(&requested)).await;
        tokio::time::sleep(Duration::from_millis(1_700)).await;
        let before_task_a_refresh =
            refresh_task_links(&cache, keys_a_b.clone(), Arc::clone(&requested)).await;
        receiver
            .changed()
            .await
            .expect("expired task-link refresh should notify");
        let after_task_a_refresh =
            refresh_task_links(&cache, keys_a_b, Arc::clone(&requested)).await;

        let expected_a = HashMap::from([(task_a.clone(), Vec::new())]);
        let expected_a_b =
            HashMap::from([(task_a.clone(), Vec::new()), (task_b.clone(), Vec::new())]);

        assert_eq!(
            (
                initial,
                reused_within_ttl,
                before_task_b_refresh,
                after_task_b_refresh,
                before_task_a_refresh,
                after_task_a_refresh,
                requested.lock().unwrap().clone(),
            ),
            (
                HashMap::new(),
                expected_a.clone(),
                expected_a.clone(),
                expected_a_b.clone(),
                expected_a_b.clone(),
                expected_a_b,
                vec![
                    HashSet::from([task_a.clone()]),
                    HashSet::from([task_b]),
                    HashSet::from([task_a]),
                ],
            ),
        );
    }

    #[rstest]
    #[case::single_wave(4, Duration::from_secs(45))]
    #[case::several_waves(14, Duration::from_secs(75))]
    #[case::measured_task_count(50, Duration::from_secs(165))]
    fn task_github_link_fetch_timeout_covers_queued_waves(
        #[case] task_count: usize,
        #[case] expected: Duration,
    ) {
        assert_eq!(task_github_link_fetch_timeout(task_count), expected);
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
        let task_metadata = SessionTaskMetadata {
            links: HashMap::from([(
                current_session.session_id.clone(),
                vec![linked_task("task-current")],
            )]),
            github_links: HashMap::from([(
                "task-current".to_string(),
                vec![
                    linked_cwd_pr,
                    tq_pr_link("linked-created-pr", 23),
                    task_only_pr,
                ],
            )]),
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
