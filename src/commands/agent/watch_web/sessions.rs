use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex};

use axum::Extension;
use axum::Json;
use axum::Router;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use chrono::{DateTime, Utc};
use serde::Serialize;

use super::EnrichmentUpdates;
use super::interrupted::interrupted_session_ids;
use super::session_enrichment::{SessionEnrichmentCache, SessionTaskMetadata};
use crate::commands::agent::proposal::ProposalBody;
use crate::commands::agent::session_title::title_display_name;
use crate::commands::agent::store;
#[cfg(test)]
use crate::commands::agent::tq_session_tasks::SessionTask as LinkedSessionTask;
use crate::commands::agent::types::{Engine, Session, SessionStatus};
use crate::commands::agent::user_turn::{UserTurnKind, user_turns};
use crate::infra::github::{PrInfo, PrState};
use crate::shared::config::{self, AgentConfig};

mod category;
use self::category::{
    SessionCategory, SessionKind, category_for_with_interruption, category_for_with_later,
    kind_for_with_todo,
};
mod sorting;
use self::sorting::sort_session_views;
mod project;
use self::project::{
    resolve_parent_project, resolve_project, resolve_repo, resolve_repos_for_sessions,
};
mod feed;
mod tree;
use self::feed::{
    SessionSnapshotCache, SessionsFeedResponse, SessionsQuery, session_feed_response,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct TqTaskView {
    id: String,
    number: u32,
    title: String,
    priority: Option<i32>,
    is_closed: bool,
    #[serde(skip)]
    priority_order: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ParentSessionView {
    id: String,
    title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct PullRequestView {
    number: u64,
    state: PrState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum SessionActivityStatus {
    Working,
    Waiting,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionView {
    id: String,
    engine: Engine,
    work_type: Option<String>,
    category: SessionCategory,
    is_paused: bool,
    activity_status: SessionActivityStatus,
    waiting_reason: Option<String>,
    waiting_for_background_tasks: bool,
    category_when_not_later: SessionCategory,
    kind: Option<SessionKind>,
    title: String,
    preview: Option<String>,
    updated_at: DateTime<Utc>,
    elapsed_seconds: u64,
    unread: bool,
    task: Option<TqTaskView>,
    tasks: Vec<TqTaskView>,
    repo: String,
    branch: Option<String>,
    can_open_in_terminal: bool,
    project: String,
    parent: Option<ParentSessionView>,
    has_session_tree: bool,
    ancestor_session_ids: Vec<String>,
    pull_request: Option<PullRequestView>,
    crit_url: Option<String>,
    later_until: Option<DateTime<Utc>>,
    is_today: bool,
    is_compacting: bool,
    decision_count: i64,
    unreviewed_decision_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionsResponse {
    revision: i64,
    work_type_nav_labels: HashMap<String, String>,
    sessions: Vec<SessionView>,
}

struct SessionSnapshot {
    revision: i64,
    sessions: Vec<Session>,
    all_sessions: Vec<Session>,
    web_states: Vec<store::WatchWebSessionState>,
    title_by_id: HashMap<String, String>,
    repo_by_id: HashMap<String, String>,
    decision_counts: HashMap<String, store::AgentDecisionCounts>,
    unanswered_proposal_titles: HashMap<String, String>,
    unanswered_todo_titles: HashMap<String, String>,
    agent_config: AgentConfig,
}

trait SessionSource: Clone + Send + Sync + 'static {
    fn load_snapshot(&self) -> impl Future<Output = anyhow::Result<SessionSnapshot>> + Send;

    fn fetch_task_metadata(
        &self,
        sessions: &[Session],
    ) -> impl Future<Output = SessionTaskMetadata> + Send;

    fn fetch_pull_requests(
        &self,
        sessions: &[Session],
    ) -> impl Future<Output = HashMap<String, Option<PrInfo>>> + Send;

    fn running_background_task_session_ids(
        &self,
        _sessions: Vec<Session>,
    ) -> impl Future<Output = anyhow::Result<HashSet<String>>> + Send {
        async { Ok(HashSet::new()) }
    }

    fn now(&self) -> DateTime<Utc>;
}

#[derive(Clone)]
struct LiveSessionSource {
    enrichment: SessionEnrichmentCache,
    message_loaders: super::messages::RouteLoaders,
}

impl SessionSource for LiveSessionSource {
    async fn load_snapshot(&self) -> anyhow::Result<SessionSnapshot> {
        tokio::task::spawn_blocking(load_snapshot).await?
    }

    async fn fetch_task_metadata(&self, sessions: &[Session]) -> SessionTaskMetadata {
        self.enrichment.task_metadata(sessions).await
    }

    async fn fetch_pull_requests(&self, sessions: &[Session]) -> HashMap<String, Option<PrInfo>> {
        self.enrichment.pull_requests(sessions).await
    }

    async fn running_background_task_session_ids(
        &self,
        sessions: Vec<Session>,
    ) -> anyhow::Result<HashSet<String>> {
        let loaders = self.message_loaders.clone();
        Ok(tokio::task::spawn_blocking(move || {
            loaders.running_background_task_session_ids(&sessions)
        })
        .await?)
    }

    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

pub(super) fn router(
    updates: EnrichmentUpdates,
    message_loaders: super::messages::RouteLoaders,
) -> Router {
    router_with_source(LiveSessionSource {
        enrichment: SessionEnrichmentCache::new(updates),
        message_loaders,
    })
}

pub(super) fn decision_category_is_done(
    session: &Session,
    agent_config: &AgentConfig,
    has_running_background_task: bool,
    state: Option<&store::WatchWebSessionState>,
    now: DateTime<Utc>,
) -> bool {
    category::decision_category_is_done(
        session,
        agent_config,
        has_running_background_task,
        state,
        now,
    )
}

fn router_with_source<S: SessionSource>(source: S) -> Router {
    router_with_source_and_cache(
        source,
        Arc::new(Mutex::new(SessionSnapshotCache::default())),
    )
}

fn router_with_source_and_cache<S: SessionSource>(
    source: S,
    snapshot_cache: Arc<Mutex<SessionSnapshotCache>>,
) -> Router {
    Router::new()
        .route("/sessions", get(list_sessions::<S>))
        .route("/sessions/{session_id}/tree", get(tree::session_tree::<S>))
        .layer(Extension(snapshot_cache))
        .with_state(source)
}

async fn list_sessions<S: SessionSource>(
    State(source): State<S>,
    Query(query): Query<SessionsQuery>,
    Extension(snapshot_cache): Extension<Arc<Mutex<SessionSnapshotCache>>>,
) -> Result<Json<SessionsFeedResponse>, StatusCode> {
    let snapshot = match source.load_snapshot().await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            tracing::warn!(%error, "failed to load agent session list");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    let mut response_sessions = snapshot.sessions.clone();
    let mut response_session_ids = response_sessions
        .iter()
        .map(|session| session.session_id.clone())
        .collect::<HashSet<_>>();
    response_sessions.extend(
        snapshot
            .all_sessions
            .iter()
            .filter(|session| {
                session.status == SessionStatus::Ended
                    && snapshot
                        .unanswered_todo_titles
                        .contains_key(&session.session_id)
            })
            .filter(|session| response_session_ids.insert(session.session_id.clone()))
            .cloned(),
    );

    let sessions_to_check = response_sessions
        .iter()
        .filter(|session| {
            session.engine == Engine::Claude && session.status == SessionStatus::Running
        })
        .cloned()
        .collect::<Vec<_>>();
    let interrupted_sessions =
        tokio::task::spawn_blocking(move || interrupted_session_ids(sessions_to_check));
    let stopped_claude_sessions = response_sessions
        .iter()
        .filter(|session| {
            session.engine == Engine::Claude && session.status == SessionStatus::Stopped
        })
        .cloned()
        .collect::<Vec<_>>();

    let (task_metadata, pull_requests, running_background_task_sessions) = tokio::join!(
        source.fetch_task_metadata(&response_sessions),
        source.fetch_pull_requests(&response_sessions),
        source.running_background_task_session_ids(stopped_claude_sessions),
    );
    let running_background_task_sessions = running_background_task_sessions.map_err(|error| {
        tracing::warn!(
            event = "agent.watch_web.background_task_lookup_failed",
            %error,
        );
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let interrupted_session_ids = interrupted_sessions
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let turn_kinds = user_turns(response_sessions.iter(), &snapshot.agent_config)
        .into_iter()
        .map(|turn| (turn.session.session_id.as_str(), turn.kind))
        .collect::<HashMap<_, _>>();
    let now = source.now();
    let state_by_id = snapshot
        .web_states
        .into_iter()
        .map(|state| (state.session_id.clone(), state))
        .collect::<HashMap<_, _>>();
    let unanswered_proposal_titles = snapshot.unanswered_proposal_titles;
    let unanswered_todo_titles = snapshot.unanswered_todo_titles;
    let tree_related_session_ids = session_tree_related_ids(&snapshot.all_sessions);

    let mut sessions = response_sessions
        .iter()
        .map(|session| {
            let tasks = task_metadata
                .links
                .get(&session.session_id)
                .into_iter()
                .flatten()
                .map(|task| TqTaskView {
                    id: task.task_id.clone(),
                    number: task.task_number,
                    title: task.task_title.clone(),
                    priority: task_metadata
                        .priorities
                        .get(&task.task_id)
                        .copied()
                        .flatten(),
                    is_closed: task.is_closed,
                    priority_order: task_metadata.priority_order.get(&task.task_id).copied(),
                })
                .collect::<Vec<_>>();
            let state = state_by_id.get(&session.session_id);
            let task = state
                .and_then(|state| state.primary_task_id.as_deref())
                .and_then(|primary_task_id| tasks.iter().find(|task| task.id == primary_task_id))
                .or_else(|| tasks.first())
                .cloned();
            let parent = session
                .ancestor_session_ids
                .last()
                .map(|id| ParentSessionView {
                    id: id.clone(),
                    title: snapshot.title_by_id.get(id).cloned(),
                });
            let repo = snapshot
                .repo_by_id
                .get(&session.session_id)
                .cloned()
                .unwrap_or_default();
            let parent_project = resolve_parent_project(
                &session.ancestor_session_ids,
                &task_metadata,
                &snapshot.repo_by_id,
            );
            let task_project = task
                .as_ref()
                .and_then(|task| task_metadata.task_projects.get(&task.id));
            let pull_request = pull_requests
                .get(&session.session_id)
                .and_then(Option::as_ref)
                .map(|info| PullRequestView {
                    number: info.number,
                    state: info.state.clone(),
                });

            let turn = turn_kinds.get(session.session_id.as_str()).copied();
            let interrupted = interrupted_session_ids.contains(&session.session_id);
            let has_running_background_task =
                running_background_task_sessions.contains(&session.session_id);
            let agent_status_state = session
                .agent_status
                .as_ref()
                .map(|status| snapshot.agent_config.status_state(&status.kind));
            let agent_is_waiting = session.status == SessionStatus::Stopped
                && agent_status_state == Some(config::AgentStatusState::Waiting);
            let proposal_title = unanswered_proposal_titles.get(&session.session_id);
            let todo_title = unanswered_todo_titles.get(&session.session_id);
            let needs_action = interrupted || proposal_title.is_some() || todo_title.is_some();
            let category_when_not_later = category_for_with_interruption(
                session,
                turn,
                needs_action,
                has_running_background_task,
                agent_status_state,
            );
            let waiting =
                agent_is_waiting && category_when_not_later == SessionCategory::InProgress;
            let category = if todo_title.is_some() {
                SessionCategory::NeedsAction
            } else {
                category_for_with_later(
                    session,
                    turn,
                    needs_action,
                    has_running_background_task,
                    agent_status_state,
                    state,
                    now,
                )
            };
            SessionView {
                id: session.session_id.clone(),
                engine: session.engine,
                work_type: session.work_type.clone(),
                category,
                is_paused: session.status == SessionStatus::Paused,
                activity_status: if session.status == SessionStatus::Running {
                    SessionActivityStatus::Working
                } else if waiting {
                    SessionActivityStatus::Waiting
                } else {
                    SessionActivityStatus::Stopped
                },
                waiting_reason: if waiting {
                    session
                        .agent_status
                        .as_ref()
                        .map(|status| status.note.trim())
                        .filter(|reason| !reason.is_empty())
                        .map(str::to_string)
                } else {
                    None
                },
                waiting_for_background_tasks: session.status == SessionStatus::Stopped
                    && has_running_background_task
                    && category_when_not_later == SessionCategory::InProgress,
                category_when_not_later,
                kind: kind_for_with_todo(
                    session,
                    turn,
                    proposal_title.is_some(),
                    todo_title.is_some(),
                    has_running_background_task,
                ),
                title: snapshot
                    .title_by_id
                    .get(&session.session_id)
                    .cloned()
                    .unwrap_or_default(),
                preview: proposal_title
                    .cloned()
                    .or_else(|| todo_title.cloned())
                    .or_else(|| session.last_message.clone()),
                updated_at: session.updated_at,
                elapsed_seconds: now
                    .signed_duration_since(session.updated_at)
                    .num_seconds()
                    .max(0) as u64,
                unread: session.is_unread_stopped(),
                task,
                tasks,
                repo: repo.clone(),
                branch: session
                    .pr_context
                    .as_ref()
                    .map(|context| context.branch.clone()),
                can_open_in_terminal: session.tmux_info.is_some(),
                project: resolve_project(parent_project, task_project.map(String::as_str), &repo),
                parent,
                has_session_tree: tree_related_session_ids.contains(&session.session_id),
                ancestor_session_ids: session.ancestor_session_ids.clone(),
                pull_request,
                crit_url: if turn_kinds.get(session.session_id.as_str())
                    == Some(&UserTurnKind::Crit)
                {
                    session.crit_urls.last().cloned()
                } else {
                    None
                },
                later_until: state.and_then(|state| state.later_until),
                is_today: state.is_some_and(|state| state.is_today),
                is_compacting: state.is_some_and(|state| state.is_compacting),
                decision_count: snapshot
                    .decision_counts
                    .get(&session.session_id)
                    .map_or(0, |counts| counts.total),
                unreviewed_decision_count: snapshot
                    .decision_counts
                    .get(&session.session_id)
                    .map_or(0, |counts| counts.unreviewed),
            }
        })
        .collect::<Vec<_>>();

    sort_session_views(&mut sessions);

    let work_type_nav_labels = snapshot
        .agent_config
        .work_types
        .iter()
        .filter_map(|(work_type, config)| {
            config
                .nav_label
                .as_ref()
                .map(|label| (work_type.clone(), label.clone()))
        })
        .collect();

    let response = SessionsResponse {
        revision: snapshot.revision,
        work_type_nav_labels,
        sessions,
    };

    Ok(Json(session_feed_response(
        &snapshot_cache,
        query.since.as_deref(),
        response,
    )?))
}

fn load_snapshot() -> anyhow::Result<SessionSnapshot> {
    let revision = store::session_store_revision()?;
    let sessions = store::list_sessions()?;
    let all_sessions = store::list_all_sessions()?;
    let title_by_id = all_sessions
        .iter()
        .map(|session| (session.session_id.clone(), title_display_name(session)))
        .collect();
    let repo_by_id = resolve_repos_for_sessions(&sessions, &all_sessions, resolve_repo);
    let sessions_dir = store::sessions_dir()?;
    let decision_counts = store::agent_decision_counts_by_session_from(&sessions_dir)?;
    let mut unanswered_proposal_titles = HashMap::new();
    let mut unanswered_todo_titles = HashMap::new();
    let visible_session_ids = all_sessions
        .iter()
        .map(|session| session.session_id.clone())
        .collect::<Vec<_>>();
    for (session_id, body) in store::unanswered_agent_proposal_bodies(&visible_session_ids)? {
        let proposal: ProposalBody = serde_json::from_str(&body)?;
        unanswered_proposal_titles
            .entry(session_id)
            .or_insert(proposal.title);
    }
    for (session_id, title) in
        store::incomplete_agent_todo_titles_from(&sessions_dir, &visible_session_ids)?
    {
        unanswered_todo_titles.entry(session_id).or_insert(title);
    }

    Ok(SessionSnapshot {
        revision,
        sessions,
        all_sessions,
        web_states: store::watch_web_session_states_from(&store::sessions_dir()?)?,
        title_by_id,
        repo_by_id,
        decision_counts,
        unanswered_proposal_titles,
        unanswered_todo_titles,
        agent_config: config::load_config_or_default().agent,
    })
}

fn session_tree_related_ids(sessions: &[Session]) -> HashSet<String> {
    let tree_sessions = sessions
        .iter()
        .filter(|session| session.status != SessionStatus::Ended)
        .cloned()
        .collect::<Vec<_>>();
    let tree_session_ids = tree_sessions
        .iter()
        .map(|session| session.session_id.as_str())
        .collect::<HashSet<_>>();
    let parent_ids = tree::session_parent_ids(&tree_sessions, &tree_session_ids);

    let mut related_ids = HashSet::new();
    for (session_id, parent_id) in parent_ids {
        if let Some(parent_id) = parent_id {
            related_ids.extend([session_id, parent_id]);
        }
    }
    related_ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::types::{AgentStatus, TmuxInfo};
    use crate::shared::config::{
        AgentStatusConfig, AgentStatusState, AgentWorkTypeColor, AgentWorkTypeConfig,
        AgentWorkTypeNamedColor,
    };
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use chrono::TimeZone;
    use rstest::{fixture, rstest};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::ServiceExt;

    fn session(status: SessionStatus) -> Session {
        Session {
            session_id: "session".to_string(),
            archived_at: None,
            pr_context: None,
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: PathBuf::from("/workspace/project"),
            transcript_path: None,
            tty: None,
            tmux_info: None::<TmuxInfo>,
            status,
            agent_status: None,
            agent_status_clear_pending: false,
            created_at: Utc.timestamp_opt(1_000, 0).unwrap(),
            updated_at: Utc.timestamp_opt(1_000, 0).unwrap(),
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

    #[derive(Clone)]
    struct TestSessionSource {
        revision: i64,
        sessions: Vec<Session>,
        all_sessions: Vec<Session>,
        web_states: Vec<store::WatchWebSessionState>,
        title_by_id: HashMap<String, String>,
        repo_by_id: HashMap<String, String>,
        unanswered_proposal_titles: HashMap<String, String>,
        unanswered_todo_titles: HashMap<String, String>,
        task_links: HashMap<String, Vec<LinkedSessionTask>>,
        task_priorities: HashMap<String, Option<i32>>,
        task_priority_order: HashMap<String, usize>,
        task_projects: HashMap<String, String>,
        pull_requests: HashMap<String, Option<PrInfo>>,
        decision_counts: HashMap<String, store::AgentDecisionCounts>,
        work_type_nav_labels: HashMap<String, String>,
        agent_statuses: HashMap<String, AgentStatusConfig>,
        message_loaders: super::super::messages::RouteLoaders,
        now: DateTime<Utc>,
    }

    impl SessionSource for TestSessionSource {
        async fn load_snapshot(&self) -> anyhow::Result<SessionSnapshot> {
            Ok(SessionSnapshot {
                revision: self.revision,
                sessions: self.sessions.clone(),
                all_sessions: self.all_sessions.clone(),
                web_states: self.web_states.clone(),
                title_by_id: self.title_by_id.clone(),
                repo_by_id: self.repo_by_id.clone(),
                decision_counts: self.decision_counts.clone(),
                unanswered_proposal_titles: self.unanswered_proposal_titles.clone(),
                unanswered_todo_titles: self.unanswered_todo_titles.clone(),
                agent_config: AgentConfig {
                    work_types: self
                        .work_type_nav_labels
                        .iter()
                        .map(|(work_type, nav_label)| {
                            (
                                work_type.clone(),
                                AgentWorkTypeConfig {
                                    icon: "◇".to_string(),
                                    color: AgentWorkTypeColor::Named(AgentWorkTypeNamedColor::Cyan),
                                    nav_label: Some(nav_label.clone()),
                                },
                            )
                        })
                        .collect(),
                    statuses: self.agent_statuses.clone(),
                    ..AgentConfig::default()
                },
            })
        }

        async fn fetch_task_metadata(&self, _sessions: &[Session]) -> SessionTaskMetadata {
            SessionTaskMetadata {
                links: self.task_links.clone(),
                priorities: self.task_priorities.clone(),
                priority_order: self.task_priority_order.clone(),
                task_projects: self.task_projects.clone(),
            }
        }

        async fn fetch_pull_requests(
            &self,
            _sessions: &[Session],
        ) -> HashMap<String, Option<PrInfo>> {
            self.pull_requests.clone()
        }

        async fn running_background_task_session_ids(
            &self,
            sessions: Vec<Session>,
        ) -> anyhow::Result<HashSet<String>> {
            let loaders = self.message_loaders.clone();
            tokio::task::spawn_blocking(move || {
                loaders.running_background_task_session_ids(&sessions)
            })
            .await
            .map_err(Into::into)
        }

        fn now(&self) -> DateTime<Utc> {
            self.now
        }
    }

    #[derive(Clone)]
    struct UpdatingTestSessionSource {
        source: TestSessionSource,
        snapshot_count: Arc<AtomicUsize>,
    }

    impl SessionSource for UpdatingTestSessionSource {
        async fn load_snapshot(&self) -> anyhow::Result<SessionSnapshot> {
            let mut snapshot = self.source.load_snapshot().await?;
            if self.snapshot_count.fetch_add(1, Ordering::Relaxed) > 0 {
                snapshot.revision += 1;
                snapshot.agent_config.work_types.insert(
                    "fixture-work-type".to_string(),
                    AgentWorkTypeConfig {
                        icon: "◇".to_string(),
                        color: AgentWorkTypeColor::Named(AgentWorkTypeNamedColor::Cyan),
                        nav_label: Some("Synthetic label".to_string()),
                    },
                );
                snapshot
                    .sessions
                    .retain(|session| session.session_id != "removed-session");
                snapshot
                    .all_sessions
                    .retain(|session| session.session_id != "removed-session");
                if let Some(session) = snapshot
                    .sessions
                    .iter_mut()
                    .find(|session| session.session_id == "child-session")
                {
                    session.last_message = Some("updated session preview".to_string());
                }
            }
            Ok(snapshot)
        }

        async fn fetch_task_metadata(&self, sessions: &[Session]) -> SessionTaskMetadata {
            self.source.fetch_task_metadata(sessions).await
        }

        async fn fetch_pull_requests(
            &self,
            sessions: &[Session],
        ) -> HashMap<String, Option<PrInfo>> {
            self.source.fetch_pull_requests(sessions).await
        }

        async fn running_background_task_session_ids(
            &self,
            sessions: Vec<Session>,
        ) -> anyhow::Result<HashSet<String>> {
            self.source
                .running_background_task_session_ids(sessions)
                .await
        }

        fn now(&self) -> DateTime<Utc> {
            self.source.now()
                + chrono::Duration::seconds(self.snapshot_count.load(Ordering::Relaxed) as i64)
        }
    }

    #[fixture]
    fn source() -> TestSessionSource {
        let mut child_session = session(SessionStatus::Stopped);
        child_session.session_id = "child-session".to_string();
        child_session.tmux_info = Some(TmuxInfo {
            session_name: "example-session".to_string(),
            window_name: "example-window".to_string(),
            window_index: 0,
            pane_id: "%1".to_string(),
            socket_path: None,
        });
        child_session.updated_at = Utc.timestamp_opt(1_000, 0).unwrap();
        child_session.last_message = Some("last message".to_string());
        child_session.ancestor_session_ids = vec!["parent-session".to_string()];
        child_session.pr_context = Some(crate::commands::agent::types::SessionPrContext {
            owner: "example-org".to_string(),
            repo: "example-repo".to_string(),
            branch: "feature-example".to_string(),
            checklist_item_id: None,
        });

        let mut parent = session(SessionStatus::Ended);
        parent.session_id = "parent-session".to_string();
        parent.ancestor_session_ids.clear();
        parent.updated_at = Utc.timestamp_opt(900, 0).unwrap();

        TestSessionSource {
            revision: 7,
            sessions: vec![child_session.clone()],
            all_sessions: vec![child_session, parent],
            web_states: vec![store::WatchWebSessionState {
                session_id: "child-session".to_string(),
                later_until: Some(Utc.timestamp_opt(4_700, 0).unwrap()),
                activity_snapshot: Some(Utc.timestamp_opt(1_000, 0).unwrap()),
                until_activity: false,
                is_today: true,
                is_compacting: false,
                primary_task_id: None,
            }],
            title_by_id: HashMap::from([
                ("child-session".to_string(), "child title".to_string()),
                ("parent-session".to_string(), "parent title".to_string()),
            ]),
            repo_by_id: HashMap::from([
                (
                    "child-session".to_string(),
                    "example-org/example-repo".to_string(),
                ),
                (
                    "parent-session".to_string(),
                    "example-org/parent-repository".to_string(),
                ),
            ]),
            unanswered_proposal_titles: HashMap::new(),
            unanswered_todo_titles: HashMap::new(),
            task_links: HashMap::from([
                (
                    "child-session".to_string(),
                    vec![LinkedSessionTask {
                        task_id: "task-id".to_string(),
                        task_number: 17,
                        task_title: "task title".to_string(),
                        parent_task_id: None,
                        is_closed: false,
                    }],
                ),
                (
                    "parent-session".to_string(),
                    vec![LinkedSessionTask {
                        task_id: "parent-task-id".to_string(),
                        task_number: 18,
                        task_title: "parent task".to_string(),
                        parent_task_id: None,
                        is_closed: false,
                    }],
                ),
            ]),
            task_priorities: HashMap::from([("task-id".to_string(), Some(1))]),
            task_priority_order: HashMap::from([("task-id".to_string(), 0)]),
            task_projects: HashMap::from([
                ("parent-task-id".to_string(), "parent project".to_string()),
                ("task-id".to_string(), "task project".to_string()),
            ]),
            pull_requests: HashMap::from([(
                "child-session".to_string(),
                Some(PrInfo {
                    number: 42,
                    title: "example pull request".to_string(),
                    state: PrState::Open,
                    url: "https://example.com/pull/42".to_string(),
                }),
            )]),
            decision_counts: HashMap::from([(
                "child-session".to_string(),
                store::AgentDecisionCounts {
                    total: 4,
                    unreviewed: 2,
                },
            )]),
            work_type_nav_labels: HashMap::new(),
            agent_statuses: HashMap::new(),
            message_loaders: super::super::messages::RouteLoaders::new(
                |_| Ok(Vec::new()),
                |_| Ok(Vec::new()),
                |_| Ok(Vec::new()),
                |_, _| Ok(()),
                |_| Ok(Vec::new()),
            ),
            now: Utc.timestamp_opt(1_100, 0).unwrap(),
        }
    }

    async fn sessions_response(source: TestSessionSource) -> (StatusCode, serde_json::Value) {
        let router = Router::new().nest("/api", router_with_source(source));
        let (status, mut response) = sessions_request(&router, "/api/sessions").await;
        if let Some(object) = response.as_object_mut() {
            object.remove("mode");
            object.remove("token");
        }
        (status, response)
    }

    async fn sessions_request(router: &Router, uri: &str) -> (StatusCode, serde_json::Value) {
        let response = router
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value = serde_json::from_slice::<serde_json::Value>(&body).unwrap();
        (status, value)
    }

    async fn session_tree_response(
        source: TestSessionSource,
        session_id: &str,
    ) -> (StatusCode, serde_json::Value) {
        let response = Router::new()
            .nest("/api", router_with_source(source))
            .oneshot(
                Request::builder()
                    .uri(format!("/api/sessions/{session_id}/tree"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value = serde_json::from_slice::<serde_json::Value>(&body).unwrap();
        (status, value)
    }

    #[tokio::test]
    async fn session_tree_route_excludes_ended_ancestors_and_keeps_active_session_data() {
        let (status, tree) = session_tree_response(source(), "child-session").await;

        assert_eq!(
            (status, tree),
            (
                StatusCode::OK,
                serde_json::json!({
                "revision": 7,
                "sessions": [
                    {
                        "id": "child-session",
                        "parentId": null,
                        "category": "later",
                        "isActive": true,
                        "kind": "Stopped",
                        "title": "child title",
                        "preview": "last message",
                        "elapsedSeconds": 100,
                        "task": {
                            "id": "task-id",
                            "number": 17,
                            "title": "task title",
                            "priority": 1,
                            "isClosed": false,
                        },
                        "repo": "example-org/example-repo",
                        "decisionCount": 4,
                        "pullRequest": { "number": 42, "state": "Open" },
                    },
                ],
                }),
            ),
        );
    }

    #[tokio::test]
    async fn session_tree_route_marks_stopped_sessions_with_pending_background_tasks_in_progress() {
        let mut source = source();
        source.all_sessions.truncate(1);
        source.all_sessions[0]
            .pending_bg_task_ids
            .insert("fixture-task-id".to_string());
        source.sessions[0]
            .pending_bg_task_ids
            .insert("fixture-task-id".to_string());
        source.web_states[0].later_until = None;

        assert_eq!(
            session_tree_response(source, "child-session").await,
            (
                StatusCode::OK,
                serde_json::json!({
                    "revision": 7,
                    "sessions": [{
                        "id": "child-session",
                        "parentId": null,
                        "category": "in_progress",
                        "isActive": true,
                        "kind": null,
                        "title": "child title",
                        "preview": "last message",
                        "elapsedSeconds": 100,
                        "task": {
                            "id": "task-id",
                            "number": 17,
                            "title": "task title",
                            "priority": 1,
                            "isClosed": false,
                        },
                        "repo": "example-org/example-repo",
                        "decisionCount": 4,
                        "pullRequest": { "number": 42, "state": "Open" },
                    }],
                }),
            ),
        );
    }

    #[tokio::test]
    async fn session_tree_route_reparents_active_descendants_across_ended_sessions() {
        let mut source = source();
        let mut grandchild = session(SessionStatus::Stopped);
        grandchild.session_id = "grandchild-session".to_string();
        grandchild.ancestor_session_ids =
            vec!["parent-session".to_string(), "child-session".to_string()];
        grandchild.updated_at = Utc.timestamp_opt(800, 0).unwrap();
        grandchild.last_message = Some("working on delegated task".to_string());
        let mut sibling = session(SessionStatus::Ended);
        sibling.session_id = "sibling-session".to_string();
        sibling.ancestor_session_ids = vec!["parent-session".to_string()];
        sibling.updated_at = Utc.timestamp_opt(700, 0).unwrap();
        source.all_sessions.extend([grandchild, sibling]);
        source.title_by_id.extend([
            (
                "grandchild-session".to_string(),
                "Grandchild session".to_string(),
            ),
            ("sibling-session".to_string(), "Sibling session".to_string()),
        ]);
        source.repo_by_id.extend([
            (
                "grandchild-session".to_string(),
                "example-org/grandchild-repo".to_string(),
            ),
            (
                "sibling-session".to_string(),
                "example-org/sibling-repo".to_string(),
            ),
        ]);

        let (status, tree) = session_tree_response(source, "child-session").await;

        assert_eq!(
            (status, tree),
            (
                StatusCode::OK,
                serde_json::json!({
                "revision": 7,
                "sessions": [
                    {
                        "id": "child-session",
                        "parentId": null,
                        "category": "later",
                        "isActive": true,
                        "kind": "Stopped",
                        "title": "child title",
                        "preview": "last message",
                        "elapsedSeconds": 100,
                        "task": {
                            "id": "task-id",
                            "number": 17,
                            "title": "task title",
                            "priority": 1,
                            "isClosed": false,
                        },
                        "repo": "example-org/example-repo",
                        "decisionCount": 4,
                        "pullRequest": { "number": 42, "state": "Open" },
                    },
                    {
                        "id": "grandchild-session",
                        "parentId": "child-session",
                        "category": "needs_action",
                        "isActive": true,
                        "kind": "Stopped",
                        "title": "Grandchild session",
                        "preview": "working on delegated task",
                        "elapsedSeconds": 300,
                        "task": null,
                        "repo": "example-org/grandchild-repo",
                        "decisionCount": 0,
                        "pullRequest": null,
                    },
                ],
                }),
            ),
        );
    }

    #[test]
    fn repo_lookup_uses_active_sessions_and_ancestors_once_per_worktree() {
        let mut active = session(SessionStatus::Running);
        active.session_id = "active-session".to_string();
        active.ancestor_session_ids = vec!["parent-session".to_string()];

        let mut parent = session(SessionStatus::Ended);
        parent.session_id = "parent-session".to_string();
        parent.cwd = active.cwd.clone();

        let mut unrelated = session(SessionStatus::Ended);
        unrelated.session_id = "unrelated-session".to_string();
        unrelated.cwd = PathBuf::from("/workspace/unrelated");

        let all_sessions = vec![active.clone(), parent, unrelated];
        let mut resolved_cwds = Vec::new();
        let repos = resolve_repos_for_sessions(&[active.clone()], &all_sessions, |cwd| {
            resolved_cwds.push(cwd.to_path_buf());
            "example-org/sample-repo".to_string()
        });

        assert_eq!(
            (repos, resolved_cwds),
            (
                HashMap::from([
                    (
                        "active-session".to_string(),
                        "example-org/sample-repo".to_string(),
                    ),
                    (
                        "parent-session".to_string(),
                        "example-org/sample-repo".to_string(),
                    ),
                ]),
                vec![active.cwd],
            ),
        );
    }

    struct ExpectedSessionDetails<'a> {
        kind: Option<&'a str>,
        category_when_not_later: &'a str,
        is_paused: bool,
        unread: bool,
        crit_url: Option<&'a str>,
    }

    struct ExpectedSessionResponse<'a> {
        engine: &'a str,
        work_type: Option<&'a str>,
        work_type_nav_labels: serde_json::Value,
        updated_at: &'a str,
        elapsed_seconds: u64,
        can_open_in_terminal: bool,
    }

    fn expected_response(
        category: &str,
        later_until: Option<&str>,
        is_today: bool,
        details: ExpectedSessionDetails<'_>,
        response: ExpectedSessionResponse<'_>,
    ) -> serde_json::Value {
        serde_json::json!({
            "revision": 7,
            "workTypeNavLabels": response.work_type_nav_labels,
            "sessions": [{
                "id": "child-session",
                "engine": response.engine,
                "workType": response.work_type,
                "category": category,
                "isPaused": details.is_paused,
                "activityStatus": if category == "in_progress" { "working" } else { "stopped" },
                "waitingReason": null,
                "waitingForBackgroundTasks": false,
                "categoryWhenNotLater": details.category_when_not_later,
                "kind": details.kind,
                "title": "child title",
                "preview": "last message",
                "updatedAt": response.updated_at,
                "elapsedSeconds": response.elapsed_seconds,
                "unread": details.unread,
                "task": {
                    "id": "task-id",
                    "number": 17,
                    "title": "task title",
                    "priority": 1,
                    "isClosed": false
                },
                "tasks": [{
                    "id": "task-id",
                    "number": 17,
                    "title": "task title",
                    "priority": 1,
                    "isClosed": false
                }],
                "repo": "example-org/example-repo",
                "branch": "feature-example",
                "canOpenInTerminal": response.can_open_in_terminal,
                "project": "parent project",
                "parent": {
                    "id": "parent-session",
                    "title": "parent title"
                },
                "hasSessionTree": false,
                "ancestorSessionIds": ["parent-session"],
                "pullRequest": {
                    "number": 42,
                    "state": "Open"
                },
                "laterUntil": later_until,
                "isToday": is_today,
                "isCompacting": false,
                "decisionCount": 4,
                "unreviewedDecisionCount": 2,
                "critUrl": details.crit_url
            }]
        })
    }

    #[rstest]
    #[tokio::test]
    async fn sessions_route_returns_the_complete_inbox_response(mut source: TestSessionSource) {
        source.web_states[0].later_until = None;
        source.web_states[0].is_today = false;
        source.sessions[0].status = SessionStatus::WaitingInput;
        source.sessions[0].crit_urls = vec!["http://127.0.0.1:6000".to_string()];
        source.sessions[0].work_type = Some("release-guide".to_string());
        source
            .work_type_nav_labels
            .insert("release-guide".to_string(), "Guides".to_string());

        assert_eq!(
            sessions_response(source).await,
            (
                StatusCode::OK,
                expected_response(
                    "needs_action",
                    None,
                    false,
                    ExpectedSessionDetails {
                        kind: Some("Review"),
                        category_when_not_later: "needs_action",
                        is_paused: false,
                        unread: false,
                        crit_url: Some("http://127.0.0.1:6000"),
                    },
                    ExpectedSessionResponse {
                        engine: "claude",
                        work_type: Some("release-guide"),
                        work_type_nav_labels: serde_json::json!({"release-guide": "Guides"}),
                        updated_at: "1970-01-01T00:16:40Z",
                        elapsed_seconds: 100,
                        can_open_in_terminal: true,
                    },
                ),
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn sessions_route_returns_deltas_for_known_tokens_and_full_data_for_unknown_tokens(
        mut source: TestSessionSource,
    ) {
        let mut removed_session = session(SessionStatus::Stopped);
        removed_session.session_id = "removed-session".to_string();
        source.sessions.push(removed_session.clone());
        source.all_sessions.push(removed_session);
        let router = Router::new().nest(
            "/api",
            router_with_source(UpdatingTestSessionSource {
                source,
                snapshot_count: Arc::new(AtomicUsize::new(0)),
            }),
        );

        let (full_status, full) = sessions_request(&router, "/api/sessions").await;
        let full_token = full["token"].as_str().unwrap();
        let (delta_status, delta) =
            sessions_request(&router, &format!("/api/sessions?since={full_token}")).await;
        let delta_token = delta["token"].as_str().unwrap();
        let (unchanged_status, unchanged) =
            sessions_request(&router, &format!("/api/sessions?since={delta_token}")).await;
        let (fallback_status, fallback) =
            sessions_request(&router, "/api/sessions?since=expired-token").await;
        let mut updated_child = full["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|session| session["id"] == "child-session")
            .unwrap()
            .clone();
        updated_child["preview"] = serde_json::json!("updated session preview");
        updated_child["elapsedSeconds"] = serde_json::json!(102);
        let mut fallback_child = updated_child.clone();
        fallback_child["elapsedSeconds"] = serde_json::json!(104);
        let mut full_without_token = full.clone();
        full_without_token.as_object_mut().unwrap().remove("token");
        let mut delta_without_token = delta.clone();
        delta_without_token.as_object_mut().unwrap().remove("token");
        let mut unchanged_without_token = unchanged.clone();
        unchanged_without_token
            .as_object_mut()
            .unwrap()
            .remove("token");
        let mut fallback_without_token = fallback.clone();
        fallback_without_token
            .as_object_mut()
            .unwrap()
            .remove("token");

        assert_eq!(
            (
                full_status,
                full_without_token,
                delta_status,
                delta_without_token,
                unchanged_status,
                unchanged_without_token,
                fallback_status,
                fallback_without_token,
            ),
            (
                StatusCode::OK,
                serde_json::json!({
                    "mode": "full",
                    "revision": 7,
                    "workTypeNavLabels": {},
                    "sessions": full["sessions"],
                }),
                StatusCode::OK,
                serde_json::json!({
                    "mode": "delta",
                    "revision": 8,
                    "workTypeNavLabels": {
                        "fixture-work-type": "Synthetic label",
                    },
                    "updatedSessions": [updated_child.clone()],
                    "removedSessionIds": ["removed-session"],
                    "sessionOrder": ["child-session"],
                }),
                StatusCode::OK,
                serde_json::json!({
                    "mode": "delta",
                    "revision": 8,
                    "updatedSessions": [],
                    "removedSessionIds": [],
                    "sessionOrder": ["child-session"],
                }),
                StatusCode::OK,
                serde_json::json!({
                    "mode": "full",
                    "revision": 8,
                    "workTypeNavLabels": {
                        "fixture-work-type": "Synthetic label",
                    },
                    "sessions": [fallback_child],
                }),
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn sessions_route_marks_tree_available_for_live_relatives_across_ended_ancestors(
        mut source: TestSessionSource,
    ) {
        source.sessions[0].status = SessionStatus::Running;
        source.all_sessions[0] = source.sessions[0].clone();
        let mut ended_intermediate = session(SessionStatus::Ended);
        ended_intermediate.session_id = "ended-intermediate-session".to_string();
        ended_intermediate.ancestor_session_ids =
            vec!["parent-session".to_string(), "child-session".to_string()];
        let mut descendant = session(SessionStatus::Running);
        descendant.session_id = "live-descendant-session".to_string();
        descendant.ancestor_session_ids = vec![
            "parent-session".to_string(),
            "child-session".to_string(),
            "ended-intermediate-session".to_string(),
        ];
        source.all_sessions.extend([ended_intermediate, descendant]);
        source.web_states[0].later_until = None;

        let mut expected = expected_response(
            "in_progress",
            None,
            true,
            ExpectedSessionDetails {
                kind: None,
                category_when_not_later: "in_progress",
                is_paused: false,
                unread: false,
                crit_url: None,
            },
            ExpectedSessionResponse {
                engine: "claude",
                work_type: None,
                work_type_nav_labels: serde_json::json!({}),
                updated_at: "1970-01-01T00:16:40Z",
                elapsed_seconds: 100,
                can_open_in_terminal: true,
            },
        );
        expected["sessions"][0]["hasSessionTree"] = serde_json::json!(true);

        assert_eq!(sessions_response(source).await, (StatusCode::OK, expected));
    }

    #[rstest]
    #[tokio::test]
    async fn sessions_route_includes_compacting_state(mut source: TestSessionSource) {
        source.sessions[0].status = SessionStatus::Running;
        source.web_states[0].later_until = None;
        source.web_states[0].is_compacting = true;

        let actual = sessions_response(source).await;
        let mut expected = expected_response(
            "in_progress",
            None,
            true,
            ExpectedSessionDetails {
                kind: None,
                category_when_not_later: "in_progress",
                is_paused: false,
                unread: false,
                crit_url: None,
            },
            ExpectedSessionResponse {
                engine: "claude",
                work_type: None,
                work_type_nav_labels: serde_json::json!({}),
                updated_at: "1970-01-01T00:16:40Z",
                elapsed_seconds: 100,
                can_open_in_terminal: true,
            },
        );
        expected["sessions"][0]["isCompacting"] = serde_json::json!(true);

        assert_eq!(actual, (StatusCode::OK, expected));
    }

    #[rstest]
    #[tokio::test]
    async fn unanswered_proposal_moves_a_running_session_to_needs_action(
        mut source: TestSessionSource,
    ) {
        source.sessions[0].status = SessionStatus::Running;
        source.web_states[0].later_until = None;
        source
            .unanswered_proposal_titles
            .insert("child-session".to_string(), "Sample proposal".to_string());

        let mut expected = expected_response(
            "needs_action",
            None,
            true,
            ExpectedSessionDetails {
                kind: Some("Proposal"),
                category_when_not_later: "needs_action",
                is_paused: false,
                unread: false,
                crit_url: None,
            },
            ExpectedSessionResponse {
                engine: "claude",
                work_type: None,
                work_type_nav_labels: serde_json::json!({}),
                updated_at: "1970-01-01T00:16:40Z",
                elapsed_seconds: 100,
                can_open_in_terminal: true,
            },
        );
        expected["sessions"][0]["activityStatus"] = serde_json::json!("working");
        expected["sessions"][0]["preview"] = serde_json::json!("Sample proposal");

        assert_eq!(sessions_response(source).await, (StatusCode::OK, expected));
    }

    #[rstest]
    #[tokio::test]
    async fn incomplete_todo_moves_a_running_session_to_needs_action(
        mut source: TestSessionSource,
    ) {
        source.sessions[0].status = SessionStatus::Running;
        source
            .unanswered_todo_titles
            .insert("child-session".to_string(), "Check the page".to_string());

        let mut expected = expected_response(
            "needs_action",
            Some("1970-01-01T01:18:20Z"),
            true,
            ExpectedSessionDetails {
                kind: Some("Todo"),
                category_when_not_later: "needs_action",
                is_paused: false,
                unread: false,
                crit_url: None,
            },
            ExpectedSessionResponse {
                engine: "claude",
                work_type: None,
                work_type_nav_labels: serde_json::json!({}),
                updated_at: "1970-01-01T00:16:40Z",
                elapsed_seconds: 100,
                can_open_in_terminal: true,
            },
        );
        expected["sessions"][0]["activityStatus"] = serde_json::json!("working");
        expected["sessions"][0]["preview"] = serde_json::json!("Check the page");

        assert_eq!(sessions_response(source).await, (StatusCode::OK, expected));
    }

    #[rstest]
    #[tokio::test]
    async fn incomplete_todo_keeps_ended_session_available_for_conversation(
        mut source: TestSessionSource,
    ) {
        source.sessions.clear();
        source.all_sessions.truncate(1);
        source.all_sessions[0].status = SessionStatus::Ended;
        source
            .unanswered_todo_titles
            .insert("child-session".to_string(), "Check the page".to_string());

        let mut expected = expected_response(
            "needs_action",
            Some("1970-01-01T01:18:20Z"),
            true,
            ExpectedSessionDetails {
                kind: Some("Todo"),
                category_when_not_later: "needs_action",
                is_paused: false,
                unread: false,
                crit_url: None,
            },
            ExpectedSessionResponse {
                engine: "claude",
                work_type: None,
                work_type_nav_labels: serde_json::json!({}),
                updated_at: "1970-01-01T00:16:40Z",
                elapsed_seconds: 100,
                can_open_in_terminal: true,
            },
        );
        expected["sessions"][0]["preview"] = serde_json::json!("Check the page");

        assert_eq!(sessions_response(source).await, (StatusCode::OK, expected));
    }

    #[rstest]
    #[case::interrupted(
        SessionStatus::Running,
        Engine::Claude,
        indoc::indoc! {r#"
            {"type":"user","message":{"content":"[Request interrupted by user]"}}
            {"type":"custom-title","customTitle":"A title"}
        "#},
        "needs_action"
    )]
    #[case::assistant_activity_after_interrupt(
        SessionStatus::Running,
        Engine::Claude,
        indoc::indoc! {r#"
            {"type":"user","message":{"content":"[Request interrupted by user]"}}
            {"type":"assistant","message":{"content":[{"type":"text","text":"Working again"}]}}
        "#},
        "in_progress"
    )]
    #[case::new_prompt_after_interrupt(
        SessionStatus::Running,
        Engine::Claude,
        indoc::indoc! {r#"
            {"type":"user","message":{"content":"[Request interrupted by user]"}}
            {"type":"user","message":{"content":"Continue the task"}}
        "#},
        "in_progress"
    )]
    #[case::queued_command_after_interrupt(
        SessionStatus::Running,
        Engine::Claude,
        indoc::indoc! {r#"
            {"type":"user","message":{"content":"[Request interrupted by user]"}}
            {"parentUuid":"fixture-turn-id","attachment":{"type":"queued_command","prompt":"Continue the task"}}
        "#},
        "in_progress"
    )]
    #[case::codex_remains_in_progress(
        SessionStatus::Running,
        Engine::Codex,
        r#"{"type":"user","message":{"content":"[Request interrupted by user]"}}"#,
        "in_progress"
    )]
    #[case::ended_claude_remains_done(
        SessionStatus::Ended,
        Engine::Claude,
        r#"{"type":"user","message":{"content":"[Request interrupted by user]"}}"#,
        "done"
    )]
    #[tokio::test]
    async fn sessions_route_only_marks_running_claude_interrupts_as_needing_action(
        mut source: TestSessionSource,
        #[case] status: SessionStatus,
        #[case] engine: Engine,
        #[case] transcript: &str,
        #[case] expected_category: &str,
    ) {
        let transcript_file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(transcript_file.path(), transcript).unwrap();
        source.sessions[0].status = status;
        source.sessions[0].engine = engine;
        source.sessions[0].transcript_path = Some(transcript_file.path().to_path_buf());
        source.web_states[0].later_until = None;

        let mut expected = expected_response(
            expected_category,
            None,
            true,
            ExpectedSessionDetails {
                kind: None,
                category_when_not_later: expected_category,
                is_paused: false,
                unread: false,
                crit_url: None,
            },
            ExpectedSessionResponse {
                engine: match engine {
                    Engine::Claude => "claude",
                    Engine::Codex => "codex",
                },
                work_type: None,
                work_type_nav_labels: serde_json::json!({}),
                updated_at: "1970-01-01T00:16:40Z",
                elapsed_seconds: 100,
                can_open_in_terminal: true,
            },
        );
        expected["sessions"][0]["activityStatus"] =
            serde_json::json!(if status == SessionStatus::Running {
                "working"
            } else {
                "stopped"
            });

        assert_eq!(sessions_response(source).await, (StatusCode::OK, expected));
    }

    #[rstest]
    #[case::without_agent_status(None, true, false)]
    #[case::configured_done_with_transcript(Some(AgentStatusState::Done), true, false)]
    #[case::waiting_with_background_task(Some(AgentStatusState::Waiting), false, false)]
    #[case::pending_task_without_transcript(None, false, false)]
    #[case::pending_task_with_parse_error(None, false, true)]
    #[tokio::test]
    async fn stopped_claude_with_running_background_task_is_in_progress(
        mut source: TestSessionSource,
        #[case] status_state: Option<AgentStatusState>,
        #[case] has_transcript: bool,
        #[case] transcript_parse_fails: bool,
    ) {
        let transcript_file = has_transcript.then(|| tempfile::NamedTempFile::new().unwrap());
        let transcript_directory = transcript_parse_fails.then(|| tempfile::tempdir().unwrap());
        if let Some(transcript_file) = &transcript_file {
            std::fs::write(
                transcript_file.path(),
                indoc::indoc! {r#"
                    {"uuid":"fixture-prompt-row","parentUuid":null,"type":"user","message":{"content":"Watch the fixture build."}}
                    {"uuid":"fixture-bash-call-row","parentUuid":"fixture-prompt-row","type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-bash-tool-use-id","name":"Bash","input":{"command":"watch fixture build","description":"Watch the synthetic build","run_in_background":true}}]}}
                    {"uuid":"fixture-bash-result-row","parentUuid":"fixture-bash-call-row","type":"user","sourceToolAssistantUUID":"fixture-bash-call-row","toolUseResult":{},"message":{"content":[{"type":"tool_result","tool_use_id":"fixture-bash-tool-use-id","content":"Command running in background with ID: fixture-task-id. Output is being written to: /fixture/task-output.log"}]}}
                "#},
            )
            .unwrap();
            source.sessions[0].transcript_path = Some(transcript_file.path().to_path_buf());
        } else {
            source.sessions[0]
                .pending_bg_task_ids
                .insert("fixture-task-id".to_string());
            source.sessions[0].transcript_path = transcript_directory
                .as_ref()
                .map(|directory| directory.path().to_path_buf());
        }
        source.web_states[0].later_until = None;
        if let Some(state) = status_state {
            let status_kind = "work_finished";
            source.sessions[0].agent_status = Some(AgentStatus {
                kind: status_kind.to_string(),
                note: "The synthetic task is complete.".to_string(),
            });
            source
                .agent_statuses
                .insert(status_kind.to_string(), AgentStatusConfig { state });
        }

        let mut expected = expected_response(
            "in_progress",
            None,
            true,
            ExpectedSessionDetails {
                kind: None,
                category_when_not_later: "in_progress",
                is_paused: false,
                unread: true,
                crit_url: None,
            },
            ExpectedSessionResponse {
                engine: "claude",
                work_type: None,
                work_type_nav_labels: serde_json::json!({}),
                updated_at: "1970-01-01T00:16:40Z",
                elapsed_seconds: 100,
                can_open_in_terminal: true,
            },
        );
        expected["sessions"][0]["activityStatus"] =
            serde_json::json!(if status_state == Some(AgentStatusState::Waiting) {
                "waiting"
            } else {
                "stopped"
            });
        expected["sessions"][0]["waitingForBackgroundTasks"] = serde_json::json!(true);
        if status_state == Some(AgentStatusState::Waiting) {
            expected["sessions"][0]["waitingReason"] =
                serde_json::json!("The synthetic task is complete.");
        }

        assert_eq!(sessions_response(source).await, (StatusCode::OK, expected));
    }

    #[rstest]
    #[tokio::test]
    async fn sessions_route_exposes_the_reason_for_a_stopped_waiting_session(
        mut source: TestSessionSource,
    ) {
        source.sessions[0].status = SessionStatus::Stopped;
        source.sessions[0].agent_status = Some(AgentStatus {
            kind: "external_wait".to_string(),
            note: "waiting for an external build".to_string(),
        });
        source.agent_statuses.insert(
            "external_wait".to_string(),
            AgentStatusConfig {
                state: AgentStatusState::Waiting,
            },
        );
        source.web_states[0].later_until = None;

        let mut expected = expected_response(
            "in_progress",
            None,
            true,
            ExpectedSessionDetails {
                kind: Some("Stopped"),
                category_when_not_later: "in_progress",
                is_paused: false,
                unread: true,
                crit_url: None,
            },
            ExpectedSessionResponse {
                engine: "claude",
                work_type: None,
                work_type_nav_labels: serde_json::json!({}),
                updated_at: "1970-01-01T00:16:40Z",
                elapsed_seconds: 100,
                can_open_in_terminal: true,
            },
        );
        expected["sessions"][0]["activityStatus"] = serde_json::json!("waiting");
        expected["sessions"][0]["waitingReason"] =
            serde_json::json!("waiting for an external build");

        assert_eq!(sessions_response(source).await, (StatusCode::OK, expected));
    }

    #[rstest]
    #[tokio::test]
    async fn sessions_route_returns_all_tasks_with_the_newest_open_task_as_primary(
        mut source: TestSessionSource,
    ) {
        source.task_links.insert(
            "child-session".to_string(),
            vec![
                LinkedSessionTask {
                    task_id: "open-task".to_string(),
                    task_number: 20,
                    task_title: "open task".to_string(),
                    parent_task_id: None,
                    is_closed: false,
                },
                LinkedSessionTask {
                    task_id: "completed-task".to_string(),
                    task_number: 19,
                    task_title: "completed task".to_string(),
                    parent_task_id: None,
                    is_closed: true,
                },
            ],
        );
        source.task_priorities = HashMap::from([
            ("completed-task".to_string(), Some(2)),
            ("open-task".to_string(), Some(1)),
        ]);

        let (_, response) = sessions_response(source).await;
        let session = &response["sessions"][0];

        assert_eq!(
            (session["task"].clone(), session["tasks"].clone()),
            (
                serde_json::json!({
                    "id": "open-task",
                    "number": 20,
                    "title": "open task",
                    "priority": 1,
                    "isClosed": false,
                }),
                serde_json::json!([
                    {
                        "id": "open-task",
                        "number": 20,
                        "title": "open task",
                        "priority": 1,
                        "isClosed": false,
                    },
                    {
                        "id": "completed-task",
                        "number": 19,
                        "title": "completed task",
                        "priority": 2,
                        "isClosed": true,
                    },
                ]),
            ),
        );
    }

    #[rstest]
    #[case::linked_primary("completed-task", "completed-task")]
    #[case::unlinked_primary_uses_existing_order("missing-task", "open-task")]
    #[tokio::test]
    async fn sessions_route_uses_a_saved_primary_only_while_it_is_linked(
        mut source: TestSessionSource,
        #[case] saved_primary_task_id: &str,
        #[case] expected_primary_task_id: &str,
    ) {
        source.web_states[0].primary_task_id = Some(saved_primary_task_id.to_string());
        source.task_links.insert(
            "child-session".to_string(),
            vec![
                LinkedSessionTask {
                    task_id: "open-task".to_string(),
                    task_number: 20,
                    task_title: "open task".to_string(),
                    parent_task_id: None,
                    is_closed: false,
                },
                LinkedSessionTask {
                    task_id: "completed-task".to_string(),
                    task_number: 19,
                    task_title: "completed task".to_string(),
                    parent_task_id: None,
                    is_closed: true,
                },
            ],
        );
        source.task_priorities = HashMap::from([
            ("completed-task".to_string(), Some(2)),
            ("open-task".to_string(), Some(1)),
        ]);

        let (status, response) = sessions_response(source).await;

        assert_eq!(
            (status, response["sessions"][0]["task"]["id"].clone()),
            (StatusCode::OK, serde_json::json!(expected_primary_task_id)),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn sessions_route_marks_paused_sessions_without_changing_their_category(
        mut source: TestSessionSource,
    ) {
        source.sessions[0].status = SessionStatus::Paused;
        source.web_states[0].later_until = None;

        assert_eq!(
            sessions_response(source).await,
            (
                StatusCode::OK,
                expected_response(
                    "done",
                    None,
                    true,
                    ExpectedSessionDetails {
                        kind: None,
                        category_when_not_later: "done",
                        is_paused: true,
                        unread: false,
                        crit_url: None,
                    },
                    ExpectedSessionResponse {
                        engine: "claude",
                        work_type: None,
                        work_type_nav_labels: serde_json::json!({}),
                        updated_at: "1970-01-01T00:16:40Z",
                        elapsed_seconds: 100,
                        can_open_in_terminal: true,
                    },
                ),
            ),
        );
    }

    #[rstest]
    #[case::with_terminal(true)]
    #[case::without_terminal(false)]
    #[tokio::test]
    async fn sessions_route_returns_saved_later_and_today_state(
        mut source: TestSessionSource,
        #[case] can_open_in_terminal: bool,
    ) {
        if !can_open_in_terminal {
            source.sessions[0].tmux_info = None;
        }

        assert_eq!(
            sessions_response(source).await,
            (
                StatusCode::OK,
                expected_response(
                    "later",
                    Some("1970-01-01T01:18:20Z"),
                    true,
                    ExpectedSessionDetails {
                        kind: Some("Stopped"),
                        category_when_not_later: "needs_action",
                        is_paused: false,
                        unread: true,
                        crit_url: None,
                    },
                    ExpectedSessionResponse {
                        engine: "claude",
                        work_type: None,
                        work_type_nav_labels: serde_json::json!({}),
                        updated_at: "1970-01-01T00:16:40Z",
                        elapsed_seconds: 100,
                        can_open_in_terminal,
                    },
                ),
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn sessions_route_restores_expired_later_sessions_to_their_category(
        mut source: TestSessionSource,
    ) {
        source.now = Utc.timestamp_opt(4_700, 0).unwrap();

        assert_eq!(
            sessions_response(source).await,
            (
                StatusCode::OK,
                expected_response(
                    "needs_action",
                    Some("1970-01-01T01:18:20Z"),
                    true,
                    ExpectedSessionDetails {
                        kind: Some("Stopped"),
                        category_when_not_later: "needs_action",
                        is_paused: false,
                        unread: true,
                        crit_url: None,
                    },
                    ExpectedSessionResponse {
                        engine: "claude",
                        work_type: None,
                        work_type_nav_labels: serde_json::json!({}),
                        updated_at: "1970-01-01T00:16:40Z",
                        elapsed_seconds: 3_700,
                        can_open_in_terminal: true,
                    },
                ),
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn sessions_route_keeps_until_activity_later_until_the_session_changes(
        mut source: TestSessionSource,
    ) {
        source.web_states[0].until_activity = true;
        source.web_states[0].later_until = None;
        let before_change = source.clone();
        source.sessions[0].updated_at = Utc.timestamp_opt(1_001, 0).unwrap();

        assert_eq!(
            (
                sessions_response(before_change).await,
                sessions_response(source).await,
            ),
            (
                (
                    StatusCode::OK,
                    expected_response(
                        "later",
                        None,
                        true,
                        ExpectedSessionDetails {
                            kind: Some("Stopped"),
                            category_when_not_later: "needs_action",
                            is_paused: false,
                            unread: true,
                            crit_url: None,
                        },
                        ExpectedSessionResponse {
                            engine: "claude",
                            work_type: None,
                            work_type_nav_labels: serde_json::json!({}),
                            updated_at: "1970-01-01T00:16:40Z",
                            elapsed_seconds: 100,
                            can_open_in_terminal: true,
                        },
                    ),
                ),
                (
                    StatusCode::OK,
                    expected_response(
                        "needs_action",
                        None,
                        true,
                        ExpectedSessionDetails {
                            kind: Some("Stopped"),
                            category_when_not_later: "needs_action",
                            is_paused: false,
                            unread: true,
                            crit_url: None,
                        },
                        ExpectedSessionResponse {
                            engine: "claude",
                            work_type: None,
                            work_type_nav_labels: serde_json::json!({}),
                            updated_at: "1970-01-01T00:16:41Z",
                            elapsed_seconds: 99,
                            can_open_in_terminal: true,
                        },
                    ),
                ),
            ),
        );
    }
}
