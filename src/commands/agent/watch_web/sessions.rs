use std::collections::{HashMap, HashSet};
use std::future::Future;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use chrono::{DateTime, Utc};
use serde::Serialize;

use super::EnrichmentUpdates;
use super::interrupted::interrupted_session_ids;
use super::session_enrichment::{SessionEnrichmentCache, SessionTaskMetadata};
use crate::commands::agent::session_title::title_display_name;
use crate::commands::agent::store;
#[cfg(test)]
use crate::commands::agent::tq_session_tasks::SessionTask as LinkedSessionTask;
use crate::commands::agent::types::{Engine, Session, SessionStatus};
use crate::commands::agent::user_turn::{UserTurnKind, user_turns};
use crate::infra::git::{github_owner_and_repo, open_repo_at};
use crate::infra::github::{PrInfo, PrState};
use crate::shared::config::{self, AgentConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
enum SessionCategory {
    NeedsAction,
    Later,
    InProgress,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
enum SessionKind {
    Question,
    Approve,
    Review,
    Error,
    Plan,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct TqTaskView {
    id: String,
    number: u32,
    title: String,
    priority: Option<i32>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionView {
    id: String,
    engine: Engine,
    work_type: Option<String>,
    category: SessionCategory,
    is_paused: bool,
    category_when_not_later: SessionCategory,
    kind: Option<SessionKind>,
    title: String,
    preview: Option<String>,
    updated_at: DateTime<Utc>,
    elapsed_seconds: u64,
    unread: bool,
    task: Option<TqTaskView>,
    repo: String,
    project: String,
    parent: Option<ParentSessionView>,
    ancestor_session_ids: Vec<String>,
    pull_request: Option<PullRequestView>,
    crit_url: Option<String>,
    later_until: Option<DateTime<Utc>>,
    is_today: bool,
    is_compacting: bool,
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
    web_states: Vec<store::WatchWebSessionState>,
    title_by_id: HashMap<String, String>,
    repo_by_id: HashMap<String, String>,
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

    fn now(&self) -> DateTime<Utc>;
}

#[derive(Clone)]
struct LiveSessionSource {
    enrichment: SessionEnrichmentCache,
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

    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

pub(super) fn router(updates: EnrichmentUpdates) -> Router {
    router_with_source(LiveSessionSource {
        enrichment: SessionEnrichmentCache::new(updates),
    })
}

fn router_with_source<S: SessionSource>(source: S) -> Router {
    Router::new()
        .route("/sessions", get(list_sessions::<S>))
        .with_state(source)
}

async fn list_sessions<S: SessionSource>(
    State(source): State<S>,
) -> Result<Json<SessionsResponse>, StatusCode> {
    let snapshot = source
        .load_snapshot()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let sessions_to_check = snapshot
        .sessions
        .iter()
        .filter(|session| {
            session.engine == Engine::Claude && session.status == SessionStatus::Running
        })
        .cloned()
        .collect::<Vec<_>>();
    let interrupted_sessions =
        tokio::task::spawn_blocking(move || interrupted_session_ids(sessions_to_check));

    let (task_metadata, pull_requests) = tokio::join!(
        source.fetch_task_metadata(&snapshot.sessions),
        source.fetch_pull_requests(&snapshot.sessions),
    );
    let interrupted_session_ids = interrupted_sessions
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let turn_kinds = user_turns(snapshot.sessions.iter(), &snapshot.agent_config)
        .into_iter()
        .map(|turn| (turn.session.session_id.as_str(), turn.kind))
        .collect::<HashMap<_, _>>();
    let now = source.now();
    let state_by_id = snapshot
        .web_states
        .into_iter()
        .map(|state| (state.session_id.clone(), state))
        .collect::<HashMap<_, _>>();

    let mut sessions = snapshot
        .sessions
        .iter()
        .map(|session| {
            let task = task_metadata
                .links
                .get(&session.session_id)
                .map(|task| TqTaskView {
                    id: task.task_id.clone(),
                    number: task.task_number,
                    title: task.task_title.clone(),
                    priority: task_metadata
                        .priorities
                        .get(&task.task_id)
                        .copied()
                        .flatten(),
                    priority_order: task_metadata.priority_order.get(&task.task_id).copied(),
                });
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
            let category_when_not_later =
                category_for_with_interruption(session, turn, interrupted);
            let state = state_by_id.get(&session.session_id);
            let category = category_for_with_later(session, turn, interrupted, state, now);
            SessionView {
                id: session.session_id.clone(),
                engine: session.engine,
                work_type: session.work_type.clone(),
                category,
                is_paused: session.status == SessionStatus::Paused,
                category_when_not_later,
                kind: kind_for(session, turn),
                title: snapshot
                    .title_by_id
                    .get(&session.session_id)
                    .cloned()
                    .unwrap_or_default(),
                preview: session.last_message.clone(),
                updated_at: session.updated_at,
                elapsed_seconds: now
                    .signed_duration_since(session.updated_at)
                    .num_seconds()
                    .max(0) as u64,
                unread: session.is_unread_stopped(),
                task,
                repo: repo.clone(),
                project: resolve_project(parent_project, task_project.map(String::as_str), &repo),
                parent,
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

    Ok(Json(SessionsResponse {
        revision: snapshot.revision,
        work_type_nav_labels,
        sessions,
    }))
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

    Ok(SessionSnapshot {
        revision,
        sessions,
        web_states: store::watch_web_session_states_from(&store::sessions_dir()?)?,
        title_by_id,
        repo_by_id,
        agent_config: config::load_config_or_default().agent,
    })
}

fn resolve_repos_for_sessions(
    sessions: &[Session],
    all_sessions: &[Session],
    mut resolve_repo: impl FnMut(&std::path::Path) -> String,
) -> HashMap<String, String> {
    let relevant_ids = sessions
        .iter()
        .flat_map(|session| {
            std::iter::once(session.session_id.clone())
                .chain(session.ancestor_session_ids.iter().cloned())
        })
        .collect::<HashSet<_>>();
    let mut repo_by_cwd = HashMap::<std::path::PathBuf, String>::new();

    all_sessions
        .iter()
        .filter(|session| relevant_ids.contains(&session.session_id))
        .map(|session| {
            let repo = repo_by_cwd
                .entry(session.cwd.clone())
                .or_insert_with(|| resolve_repo(&session.cwd))
                .clone();
            (session.session_id.clone(), repo)
        })
        .collect()
}

fn resolve_repo(cwd: &std::path::Path) -> String {
    if let Ok(repo) = open_repo_at(cwd)
        && let Ok((owner, repo)) = github_owner_and_repo(&repo)
    {
        return format!("{owner}/{repo}");
    }

    cwd.file_name()
        .and_then(|name| name.to_str())
        .map(String::from)
        .unwrap_or_else(|| cwd.display().to_string())
}

fn resolve_project(parent_project: Option<&str>, task_project: Option<&str>, repo: &str) -> String {
    parent_project
        .filter(|project| !project.trim().is_empty())
        .or_else(|| task_project.filter(|project| !project.trim().is_empty()))
        .or_else(|| (!repo.trim().is_empty()).then_some(repo))
        .unwrap_or("Unsorted")
        .to_string()
}

fn resolve_parent_project<'a>(
    ancestor_session_ids: &[String],
    task_metadata: &'a SessionTaskMetadata,
    repo_by_id: &'a HashMap<String, String>,
) -> Option<&'a str> {
    ancestor_session_ids.iter().find_map(|ancestor_id| {
        task_metadata
            .links
            .get(ancestor_id)
            .and_then(|task| task_metadata.task_projects.get(&task.task_id))
            .map(String::as_str)
            .filter(|project| !project.trim().is_empty())
            .or_else(|| {
                repo_by_id
                    .get(ancestor_id)
                    .map(String::as_str)
                    .filter(|repo| !repo.trim().is_empty())
            })
    })
}

#[cfg(test)]
mod project_tests {
    use super::{SessionTaskMetadata, resolve_parent_project, resolve_project};
    use crate::commands::agent::tq_session_tasks::SessionTask;
    use rstest::rstest;
    use std::collections::HashMap;

    fn task_metadata(links: &[(&str, &str)], projects: &[(&str, &str)]) -> SessionTaskMetadata {
        SessionTaskMetadata {
            links: links
                .iter()
                .map(|(session_id, task_id)| {
                    (
                        (*session_id).to_string(),
                        SessionTask {
                            task_id: (*task_id).to_string(),
                            task_number: 1,
                            task_title: "Sample task".to_string(),
                            parent_task_id: None,
                            is_closed: false,
                        },
                    )
                })
                .collect(),
            task_projects: projects
                .iter()
                .map(|(task_id, project)| ((*task_id).to_string(), (*project).to_string()))
                .collect(),
            ..SessionTaskMetadata::default()
        }
    }

    #[rstest]
    #[case::parent_takes_precedence(
        Some("parent-project"),
        Some("task-project"),
        "example-org/repo",
        "parent-project"
    )]
    #[case::task_project_precedes_repo(
        None,
        Some("task-project"),
        "example-org/repo",
        "task-project"
    )]
    #[case::repo_is_the_fallback(None, None, "example-org/repo", "example-org/repo")]
    #[case::unsorted_is_the_final_fallback(None, None, "", "Unsorted")]
    #[case::empty_candidates_are_ignored(
        Some(" "),
        Some(""),
        "example-org/repo",
        "example-org/repo"
    )]
    fn resolves_projects_by_source_precedence(
        #[case] parent_project: Option<&str>,
        #[case] task_project: Option<&str>,
        #[case] repo: &str,
        #[case] expected: &str,
    ) {
        assert_eq!(
            resolve_project(parent_project, task_project, repo),
            expected
        );
    }

    #[rstest]
    #[case::single_parent_uses_its_task_project(
        vec!["parent"],
        vec![("parent", "parent-task")],
        vec![("parent-task", "parent-project")],
        vec![],
        Some("parent-project")
    )]
    #[case::single_parent_falls_back_to_its_repo(
        vec!["parent"],
        vec![],
        vec![],
        vec![("parent", "sample/parent-repo")],
        Some("sample/parent-repo")
    )]
    #[case::three_levels_use_the_root_project_first(
        vec!["root", "middle", "parent"],
        vec![("root", "root-task")],
        vec![("root-task", "root-project")],
        vec![
            ("root", "sample/root-repo"),
            ("middle", "sample/middle-repo"),
            ("parent", "sample/parent-repo"),
        ],
        Some("root-project")
    )]
    #[case::unknown_root_uses_the_next_available_ancestor(
        vec!["unknown-root", "middle", "parent"],
        vec![("middle", "middle-task")],
        vec![("middle-task", "middle-project")],
        vec![("parent", "sample/parent-repo")],
        Some("middle-project")
    )]
    #[case::unknown_ancestors_fall_back_to_the_current_session(
        vec!["unknown-root", "unknown-parent"],
        vec![],
        vec![],
        vec![],
        None
    )]
    fn resolves_parent_project_from_root_to_parent(
        #[case] ancestor_ids: Vec<&str>,
        #[case] links: Vec<(&str, &str)>,
        #[case] projects: Vec<(&str, &str)>,
        #[case] repos: Vec<(&str, &str)>,
        #[case] expected: Option<&str>,
    ) {
        let ancestor_ids = ancestor_ids
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let metadata = task_metadata(&links, &projects);
        let repo_by_id = repos
            .into_iter()
            .map(|(session_id, repo)| (session_id.to_string(), repo.to_string()))
            .collect::<HashMap<_, _>>();

        assert_eq!(
            resolve_parent_project(&ancestor_ids, &metadata, &repo_by_id),
            expected,
        );
    }
}

fn category_for(session: &Session, turn: Option<UserTurnKind>) -> SessionCategory {
    if turn.is_some()
        || session.status == SessionStatus::Stopped
        || session
            .current_tool
            .as_deref()
            .is_some_and(|tool| tool.starts_with("ExitPlanMode"))
    {
        SessionCategory::NeedsAction
    } else if session.status == SessionStatus::Running {
        SessionCategory::InProgress
    } else {
        SessionCategory::Done
    }
}

fn category_for_with_later(
    session: &Session,
    turn: Option<UserTurnKind>,
    interrupted: bool,
    state: Option<&store::WatchWebSessionState>,
    now: DateTime<Utc>,
) -> SessionCategory {
    let still_unchanged = state.is_some_and(|state| {
        state
            .activity_snapshot
            .is_none_or(|snapshot| session.updated_at <= snapshot)
    });
    let later_is_active = still_unchanged
        && state.is_some_and(|state| {
            state.until_activity || state.later_until.is_some_and(|deadline| now < deadline)
        });
    if later_is_active {
        SessionCategory::Later
    } else {
        category_for_with_interruption(session, turn, interrupted)
    }
}

fn category_for_with_interruption(
    session: &Session,
    turn: Option<UserTurnKind>,
    interrupted: bool,
) -> SessionCategory {
    if interrupted {
        SessionCategory::NeedsAction
    } else {
        category_for(session, turn)
    }
}

fn kind_for(session: &Session, turn: Option<UserTurnKind>) -> Option<SessionKind> {
    if session
        .current_tool
        .as_deref()
        .is_some_and(|tool| tool.starts_with("ExitPlanMode"))
    {
        return Some(SessionKind::Plan);
    }

    let Some(turn) = turn else {
        return (session.status == SessionStatus::Stopped).then_some(SessionKind::Stopped);
    };

    match turn {
        UserTurnKind::Ask => Some(SessionKind::Question),
        UserTurnKind::Allow => Some(SessionKind::Approve),
        UserTurnKind::Crit | UserTurnKind::Body => Some(SessionKind::Review),
        UserTurnKind::Failed => Some(SessionKind::Error),
        UserTurnKind::AgentStatus => {
            let status = session.agent_status.as_ref()?;
            let content = format!("{} {}", status.kind, status.note).to_ascii_lowercase();
            let has_word = |cues: &[&str]| {
                content
                    .split(|character: char| !character.is_ascii_alphanumeric())
                    .any(|word| cues.contains(&word))
            };
            if content.contains('?') || has_word(&["question", "ask", "asked", "answer", "reply"]) {
                Some(SessionKind::Question)
            } else if has_word(&[
                "review",
                "reviews",
                "reviewing",
                "crit",
                "body",
                "approve",
                "approval",
                "proposal",
            ]) {
                Some(SessionKind::Review)
            } else {
                Some(SessionKind::Stopped)
            }
        }
        UserTurnKind::Merge => Some(SessionKind::Review),
    }
}

fn compare_tasks(left: &Option<TqTaskView>, right: &Option<TqTaskView>) -> std::cmp::Ordering {
    match (left, right) {
        (Some(left), Some(right)) => left
            .priority_order
            .unwrap_or(usize::MAX)
            .cmp(&right.priority_order.unwrap_or(usize::MAX)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

fn sort_session_views(sessions: &mut [SessionView]) {
    sessions.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then_with(|| compare_tasks(&left.task, &right.task))
            .then_with(|| right.updated_at.cmp(&left.updated_at))
            .then_with(|| left.id.cmp(&right.id))
    });
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

    fn row(
        id: &str,
        category: SessionCategory,
        task: Option<(Option<i32>, Option<usize>, u32)>,
        updated_at: i64,
    ) -> SessionView {
        SessionView {
            id: id.to_string(),
            engine: Engine::Claude,
            work_type: None,
            category,
            is_paused: false,
            category_when_not_later: category,
            kind: None,
            title: id.to_string(),
            preview: None,
            updated_at: Utc.timestamp_opt(updated_at, 0).unwrap(),
            elapsed_seconds: 0,
            unread: false,
            task: task.map(|(priority, priority_order, number)| TqTaskView {
                id: format!("task-{number}"),
                number,
                title: id.to_string(),
                priority,
                priority_order,
            }),
            repo: String::new(),
            project: "Unsorted".to_string(),
            parent: None,
            ancestor_session_ids: Vec::new(),
            pull_request: None,
            crit_url: None,
            later_until: None,
            is_today: false,
            is_compacting: false,
        }
    }

    #[derive(Clone)]
    struct TestSessionSource {
        revision: i64,
        sessions: Vec<Session>,
        web_states: Vec<store::WatchWebSessionState>,
        title_by_id: HashMap<String, String>,
        repo_by_id: HashMap<String, String>,
        task_links: HashMap<String, LinkedSessionTask>,
        task_priorities: HashMap<String, Option<i32>>,
        task_priority_order: HashMap<String, usize>,
        task_projects: HashMap<String, String>,
        pull_requests: HashMap<String, Option<PrInfo>>,
        work_type_nav_labels: HashMap<String, String>,
        now: DateTime<Utc>,
    }

    impl SessionSource for TestSessionSource {
        async fn load_snapshot(&self) -> anyhow::Result<SessionSnapshot> {
            Ok(SessionSnapshot {
                revision: self.revision,
                sessions: self.sessions.clone(),
                web_states: self.web_states.clone(),
                title_by_id: self.title_by_id.clone(),
                repo_by_id: self.repo_by_id.clone(),
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

        fn now(&self) -> DateTime<Utc> {
            self.now
        }
    }

    #[fixture]
    fn source() -> TestSessionSource {
        let mut session = session(SessionStatus::Stopped);
        session.session_id = "child-session".to_string();
        session.updated_at = Utc.timestamp_opt(1_000, 0).unwrap();
        session.last_message = Some("last message".to_string());
        session.ancestor_session_ids = vec!["parent-session".to_string()];
        session.pr_context = Some(crate::commands::agent::types::SessionPrContext {
            owner: "example-org".to_string(),
            repo: "example-repo".to_string(),
            branch: "feature-example".to_string(),
            checklist_item_id: None,
        });

        TestSessionSource {
            revision: 7,
            sessions: vec![session],
            web_states: vec![store::WatchWebSessionState {
                session_id: "child-session".to_string(),
                later_until: Some(Utc.timestamp_opt(4_700, 0).unwrap()),
                activity_snapshot: Some(Utc.timestamp_opt(1_000, 0).unwrap()),
                until_activity: false,
                is_today: true,
                is_compacting: false,
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
            task_links: HashMap::from([
                (
                    "child-session".to_string(),
                    LinkedSessionTask {
                        task_id: "task-id".to_string(),
                        task_number: 17,
                        task_title: "task title".to_string(),
                        parent_task_id: None,
                        is_closed: false,
                    },
                ),
                (
                    "parent-session".to_string(),
                    LinkedSessionTask {
                        task_id: "parent-task-id".to_string(),
                        task_number: 18,
                        task_title: "parent task".to_string(),
                        parent_task_id: None,
                        is_closed: false,
                    },
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
            work_type_nav_labels: HashMap::new(),
            now: Utc.timestamp_opt(1_100, 0).unwrap(),
        }
    }

    async fn sessions_response(source: TestSessionSource) -> (StatusCode, serde_json::Value) {
        let response = Router::new()
            .nest("/api", router_with_source(source))
            .oneshot(
                Request::builder()
                    .uri("/api/sessions")
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
                    "priority": 1
                },
                "repo": "example-org/example-repo",
                "project": "parent project",
                "parent": {
                    "id": "parent-session",
                    "title": "parent title"
                },
                "ancestorSessionIds": ["parent-session"],
                "pullRequest": {
                    "number": 42,
                    "state": "Open"
                },
                "laterUntil": later_until,
                "isToday": is_today,
                "isCompacting": false,
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
                    },
                ),
            ),
        );
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
            },
        );
        expected["sessions"][0]["isCompacting"] = serde_json::json!(true);

        assert_eq!(actual, (StatusCode::OK, expected));
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

        assert_eq!(
            sessions_response(source).await,
            (
                StatusCode::OK,
                expected_response(
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
                    },
                ),
            ),
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
                    },
                ),
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn sessions_route_returns_saved_later_and_today_state(source: TestSessionSource) {
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
                        },
                    ),
                ),
            ),
        );
    }

    #[rstest]
    #[case::waiting_tool(UserTurnKind::Ask, Some(SessionKind::Question))]
    #[case::permission(UserTurnKind::Allow, Some(SessionKind::Approve))]
    #[case::crit_review(UserTurnKind::Crit, Some(SessionKind::Review))]
    #[case::draft_review(UserTurnKind::Body, Some(SessionKind::Review))]
    #[case::failure(UserTurnKind::Failed, Some(SessionKind::Error))]
    #[case::merge(UserTurnKind::Merge, Some(SessionKind::Review))]
    fn maps_user_turn_kinds_to_web_kinds(
        #[case] turn: UserTurnKind,
        #[case] expected: Option<SessionKind>,
    ) {
        assert_eq!(
            kind_for(&session(SessionStatus::WaitingInput), Some(turn)),
            expected
        );
    }

    #[rstest]
    #[case::agent_question("ask_user", "", SessionKind::Question)]
    #[case::note_question("needs_attention", "Which option?", SessionKind::Question)]
    #[case::agent_review("needs_review", "", SessionKind::Review)]
    #[case::task_in_review_note("needs_review", "Review the task plan", SessionKind::Review)]
    #[case::agent_stopped("waiting_for_external", "", SessionKind::Stopped)]
    fn maps_agent_status_to_web_kind(
        #[case] status_kind: &str,
        #[case] note: &str,
        #[case] expected: SessionKind,
    ) {
        let mut session = session(SessionStatus::Stopped);
        session.agent_status = Some(AgentStatus {
            kind: status_kind.to_string(),
            note: note.to_string(),
        });

        assert_eq!(
            kind_for(&session, Some(UserTurnKind::AgentStatus)),
            Some(expected)
        );
    }

    #[test]
    fn maps_exit_plan_mode_to_plan_kind() {
        let mut session = session(SessionStatus::WaitingInput);
        session.current_tool = Some("ExitPlanMode".to_string());

        assert_eq!(
            kind_for(&session, Some(UserTurnKind::Ask)),
            Some(SessionKind::Plan)
        );
    }

    #[test]
    fn exit_plan_mode_needs_action() {
        let mut session = session(SessionStatus::WaitingInput);
        session.current_tool = Some("ExitPlanMode".to_string());

        assert_eq!(category_for(&session, None), SessionCategory::NeedsAction);
    }

    #[test]
    fn stopped_without_a_user_turn_has_stopped_kind() {
        assert_eq!(
            kind_for(&session(SessionStatus::Stopped), None),
            Some(SessionKind::Stopped)
        );
    }

    #[rstest]
    #[case::stopped(SessionStatus::Stopped, None, SessionCategory::NeedsAction)]
    #[case::user_turn(
        SessionStatus::WaitingInput,
        Some(UserTurnKind::Ask),
        SessionCategory::NeedsAction
    )]
    #[case::running(SessionStatus::Running, None, SessionCategory::InProgress)]
    #[case::paused(SessionStatus::Paused, None, SessionCategory::Done)]
    #[case::ended(SessionStatus::Ended, None, SessionCategory::Done)]
    fn maps_sessions_to_inbox_categories(
        #[case] status: SessionStatus,
        #[case] turn: Option<UserTurnKind>,
        #[case] expected: SessionCategory,
    ) {
        assert_eq!(category_for(&session(status), turn), expected);
    }

    #[rstest]
    #[case::configured_waiting("waiting_for_external", AgentStatusState::Waiting)]
    #[case::configured_done("work_finished", AgentStatusState::Done)]
    fn stopped_agent_statuses_remain_in_needs_action(
        #[case] status_kind: &str,
        #[case] state: AgentStatusState,
    ) {
        let mut session = session(SessionStatus::Stopped);
        session.agent_status = Some(AgentStatus {
            kind: status_kind.to_string(),
            note: "waiting for a task result".to_string(),
        });
        let mut agent_config = AgentConfig::default();
        agent_config
            .statuses
            .insert(status_kind.to_string(), AgentStatusConfig { state });
        let turn = user_turns([&session], &agent_config)
            .first()
            .map(|turn| turn.kind);

        assert_eq!(
            (turn, category_for(&session, turn), kind_for(&session, turn)),
            (
                None,
                SessionCategory::NeedsAction,
                Some(SessionKind::Stopped)
            ),
        );
    }

    #[rstest]
    #[case::lower_priority_first(Some((Some(1), Some(0), 1)), Some((Some(0), Some(1), 2)), std::cmp::Ordering::Less)]
    #[case::unset_before_deferred(Some((None, Some(3), 1)), Some((Some(-1), Some(4), 2)), std::cmp::Ordering::Less)]
    #[case::deferred_after_regular(Some((Some(-1), Some(4), 1)), Some((Some(0), Some(2), 2)), std::cmp::Ordering::Greater)]
    #[case::missing_order_after_known(Some((Some(1), None, 1)), Some((Some(-1), Some(4), 2)), std::cmp::Ordering::Greater)]
    #[case::same_priority_ties(Some((Some(1), Some(2), 1)), Some((Some(1), Some(2), 2)), std::cmp::Ordering::Equal)]
    #[case::taskless_last(Some((Some(1), Some(2), 1)), None, std::cmp::Ordering::Less)]
    fn compares_task_priority(
        #[case] left: Option<(Option<i32>, Option<usize>, u32)>,
        #[case] right: Option<(Option<i32>, Option<usize>, u32)>,
        #[case] expected: std::cmp::Ordering,
    ) {
        let to_task = |task: Option<(Option<i32>, Option<usize>, u32)>| {
            task.map(|(priority, priority_order, number)| TqTaskView {
                id: "task".to_string(),
                number,
                title: "task".to_string(),
                priority,
                priority_order,
            })
        };
        assert_eq!(compare_tasks(&to_task(left), &to_task(right)), expected);
    }

    #[rstest]
    #[case::category_then_priority(
        vec![
            row("done", SessionCategory::Done, Some((Some(0), Some(0), 1)), 1),
            row("working", SessionCategory::InProgress, Some((Some(0), Some(0), 2)), 1),
            row("needs_low", SessionCategory::NeedsAction, Some((Some(2), Some(1), 3)), 1),
            row("needs_high", SessionCategory::NeedsAction, Some((Some(1), Some(0), 4)), 1),
            row("needs_without_task", SessionCategory::NeedsAction, None, 0),
        ],
        vec!["needs_high", "needs_low", "needs_without_task", "working", "done"]
    )]
    #[case::unset_before_deferred(
        vec![
            row("deferred", SessionCategory::NeedsAction, Some((Some(-1), Some(4), 2)), 1),
            row("inbox", SessionCategory::NeedsAction, Some((None, Some(3), 1)), 1),
        ],
        vec!["inbox", "deferred"]
    )]
    #[case::newest_same_priority_first(
        vec![
            row("newer", SessionCategory::NeedsAction, Some((Some(1), Some(0), 1)), 20),
            row("older", SessionCategory::NeedsAction, Some((Some(1), Some(0), 2)), 10),
        ],
        vec!["newer", "older"]
    )]
    #[case::newest_without_task_first(
        vec![
            row("newer", SessionCategory::NeedsAction, None, 20),
            row("older", SessionCategory::NeedsAction, None, 10),
        ],
        vec!["newer", "older"]
    )]
    #[case::id_breaks_equal_timestamps(
        vec![
            row("session_b", SessionCategory::NeedsAction, Some((Some(1), Some(0), 1)), 20),
            row("session_a", SessionCategory::NeedsAction, Some((Some(1), Some(0), 2)), 20),
        ],
        vec!["session_a", "session_b"]
    )]
    fn orders_session_rows(#[case] mut rows: Vec<SessionView>, #[case] expected_ids: Vec<&str>) {
        sort_session_views(&mut rows);

        assert_eq!(
            rows.into_iter()
                .map(|session| session.id)
                .collect::<Vec<_>>(),
            expected_ids
        );
    }
}
