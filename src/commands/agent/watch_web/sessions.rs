use std::collections::HashMap;
use std::future::Future;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use chrono::{DateTime, Utc};
use serde::Serialize;

use super::EnrichmentUpdates;
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
    category: SessionCategory,
    category_when_not_later: SessionCategory,
    kind: Option<SessionKind>,
    title: String,
    preview: Option<String>,
    updated_at: DateTime<Utc>,
    elapsed_seconds: u64,
    unread: bool,
    task: Option<TqTaskView>,
    repo: String,
    parent: Option<ParentSessionView>,
    pull_request: Option<PullRequestView>,
    later_until: Option<DateTime<Utc>>,
    is_today: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionsResponse {
    revision: i64,
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

    let (task_metadata, pull_requests) = tokio::join!(
        source.fetch_task_metadata(&snapshot.sessions),
        source.fetch_pull_requests(&snapshot.sessions),
    );
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
            let pull_request = pull_requests
                .get(&session.session_id)
                .and_then(Option::as_ref)
                .map(|info| PullRequestView {
                    number: info.number,
                    state: info.state.clone(),
                });

            let turn = turn_kinds.get(session.session_id.as_str()).copied();
            let category_when_not_later = category_for(session, turn);
            let state = state_by_id.get(&session.session_id);
            let category = category_for_with_later(session, turn, state, now);
            SessionView {
                id: session.session_id.clone(),
                engine: session.engine,
                category,
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
                repo: snapshot
                    .repo_by_id
                    .get(&session.session_id)
                    .cloned()
                    .unwrap_or_default(),
                parent,
                pull_request,
                later_until: state.and_then(|state| state.later_until),
                is_today: state.is_some_and(|state| state.is_today),
            }
        })
        .collect::<Vec<_>>();

    sort_session_views(&mut sessions);

    Ok(Json(SessionsResponse {
        revision: snapshot.revision,
        sessions,
    }))
}

fn load_snapshot() -> anyhow::Result<SessionSnapshot> {
    let revision = store::session_store_revision()?;
    let sessions = store::list_sessions()?;
    let title_by_id = store::list_all_sessions()?
        .into_iter()
        .map(|session| (session.session_id.clone(), title_display_name(&session)))
        .collect();
    let repo_by_id = sessions
        .iter()
        .map(|session| (session.session_id.clone(), resolve_repo(&session.cwd)))
        .collect();

    Ok(SessionSnapshot {
        revision,
        sessions,
        web_states: store::watch_web_session_states_from(&store::sessions_dir()?)?,
        title_by_id,
        repo_by_id,
        agent_config: config::load_config_or_default().agent,
    })
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
            .then_with(|| left.updated_at.cmp(&right.updated_at))
            .then_with(|| left.id.cmp(&right.id))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::types::{AgentStatus, TmuxInfo};
    use crate::shared::config::{AgentStatusConfig, AgentStatusState};
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
            category,
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
            parent: None,
            pull_request: None,
            later_until: None,
            is_today: false,
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
        pull_requests: HashMap<String, Option<PrInfo>>,
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
                agent_config: AgentConfig::default(),
            })
        }

        async fn fetch_task_metadata(&self, _sessions: &[Session]) -> SessionTaskMetadata {
            SessionTaskMetadata {
                links: self.task_links.clone(),
                priorities: self.task_priorities.clone(),
                priority_order: self.task_priority_order.clone(),
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
            }],
            title_by_id: HashMap::from([
                ("child-session".to_string(), "child title".to_string()),
                ("parent-session".to_string(), "parent title".to_string()),
            ]),
            repo_by_id: HashMap::from([(
                "child-session".to_string(),
                "example-org/example-repo".to_string(),
            )]),
            task_links: HashMap::from([(
                "child-session".to_string(),
                LinkedSessionTask {
                    task_id: "task-id".to_string(),
                    task_number: 17,
                    task_title: "task title".to_string(),
                    parent_task_id: None,
                    is_closed: false,
                },
            )]),
            task_priorities: HashMap::from([("task-id".to_string(), Some(1))]),
            task_priority_order: HashMap::from([("task-id".to_string(), 0)]),
            pull_requests: HashMap::from([(
                "child-session".to_string(),
                Some(PrInfo {
                    number: 42,
                    title: "example pull request".to_string(),
                    state: PrState::Open,
                    url: "https://example.com/pull/42".to_string(),
                }),
            )]),
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

    fn expected_response(
        category: &str,
        later_until: Option<&str>,
        is_today: bool,
        updated_at: &str,
        elapsed_seconds: u64,
    ) -> serde_json::Value {
        serde_json::json!({
            "revision": 7,
            "sessions": [{
                "id": "child-session",
                "engine": "claude",
                "category": category,
                "categoryWhenNotLater": "needs_action",
                "kind": "Stopped",
                "title": "child title",
                "preview": "last message",
                "updatedAt": updated_at,
                "elapsedSeconds": elapsed_seconds,
                "unread": true,
                "task": {
                    "id": "task-id",
                    "number": 17,
                    "title": "task title",
                    "priority": 1
                },
                "repo": "example-org/example-repo",
                "parent": {
                    "id": "parent-session",
                    "title": "parent title"
                },
                "pullRequest": {
                    "number": 42,
                    "state": "Open"
                },
                "laterUntil": later_until,
                "isToday": is_today
            }]
        })
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
                    "1970-01-01T00:16:40Z",
                    100,
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
                    "1970-01-01T00:16:40Z",
                    3_700,
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
                    expected_response("later", None, true, "1970-01-01T00:16:40Z", 100,),
                ),
                (
                    StatusCode::OK,
                    expected_response("needs_action", None, true, "1970-01-01T00:16:41Z", 99,),
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
    #[case::oldest_waiting_first(
        vec![
            row("newer", SessionCategory::NeedsAction, Some((Some(1), Some(0), 1)), 20),
            row("older", SessionCategory::NeedsAction, Some((Some(1), Some(0), 2)), 10),
        ],
        vec!["older", "newer"]
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
