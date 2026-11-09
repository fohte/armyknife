use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::{Path as RoutePath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::super::claude_messaging;
use super::super::claude_registry;
use super::super::codex_steer;
use super::super::resume::SHELL_COMMANDS;
use super::super::store;
use super::super::types::{Engine, Session, SessionStatus, TmuxInfo};
use super::super::user_turn::is_ask_user_question;

pub(super) fn router() -> Router {
    router_with_state(ActionState {
        sessions_dir: None,
        runtime: Arc::new(LiveActionRuntime),
    })
}

fn router_with_state(state: ActionState) -> Router {
    Router::new()
        .route("/sessions/{id}/actions", get(actions))
        .route("/sessions/{id}/messages", post(send_message))
        .route("/sessions/{id}/answer", post(answer_question))
        .route(
            "/sessions/{id}/permission/approve",
            post(approve_permission),
        )
        .route("/sessions/{id}/permission/reject", post(reject_permission))
        .route("/sessions/{id}/interrupt", post(interrupt))
        .route("/sessions/{id}/focus", post(focus))
        .route("/sessions/{id}/read", post(mark_read))
        .route("/sessions/{id}/archive", post(archive))
        .with_state(state)
}

#[derive(Clone)]
struct ActionState {
    sessions_dir: Option<PathBuf>,
    runtime: Arc<dyn ActionRuntime>,
}

trait ActionRuntime: Send + Sync {
    fn send_message(&self, session: &Session, content: &str) -> Result<()>;
    fn pane_current_command(&self, tmux_info: &TmuxInfo) -> Option<String>;
    fn send_keys_if_agent_is_running(
        &self,
        tmux_info: &TmuxInfo,
        engine: Engine,
        current_command: &str,
        keys: &[&str],
    ) -> Result<bool>;
    fn focus(&self, tmux_info: &TmuxInfo) -> Result<()>;
}

struct LiveActionRuntime;

impl ActionRuntime for LiveActionRuntime {
    fn send_message(&self, session: &Session, content: &str) -> Result<()> {
        match session.engine {
            Engine::Claude => {
                let connection = claude_registry::load_peer_connection(&session.session_id)
                    .context("no active Claude Code session registry entry")?;
                let socket_path = connection
                    .messaging_socket_path
                    .context("Claude Code session does not expose a messaging socket")?;
                claude_messaging::send_message(&socket_path, connection.pid, content)
            }
            Engine::Codex => {
                codex_steer::send_message(&session.session_id, content).map_err(anyhow::Error::from)
            }
        }
    }

    fn pane_current_command(&self, tmux_info: &TmuxInfo) -> Option<String> {
        crate::infra::tmux::get_pane_current_command_for_socket(
            tmux_info.socket_path.as_deref(),
            &tmux_info.pane_id,
        )
    }

    fn send_keys_if_agent_is_running(
        &self,
        tmux_info: &TmuxInfo,
        engine: Engine,
        current_command: &str,
        keys: &[&str],
    ) -> Result<bool> {
        let Some(pane_pid) = crate::infra::tmux::get_pane_pid_for_socket(
            tmux_info.socket_path.as_deref(),
            &tmux_info.pane_id,
        )
        .ok()
        .flatten() else {
            return Ok(false);
        };
        let Some(process_snapshot) = crate::infra::process::ProcessSnapshot::capture() else {
            return Ok(false);
        };
        if !super::super::pane::process::pane_has_live_agent_process(
            pane_pid,
            engine,
            Some(&process_snapshot),
        ) {
            return Ok(false);
        }

        crate::infra::tmux::send_keys_if_pane_command_matches_for_socket(
            tmux_info.socket_path.as_deref(),
            &tmux_info.pane_id,
            current_command,
            keys,
        )
        .map_err(anyhow::Error::from)
    }

    fn focus(&self, tmux_info: &TmuxInfo) -> Result<()> {
        crate::infra::tmux::focus_pane_for_socket(
            tmux_info.socket_path.as_deref(),
            &tmux_info.pane_id,
        )
        .map_err(anyhow::Error::from)
    }
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: "session not found".to_string(),
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }

    fn operation(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: error.to_string(),
        }
    }

    fn internal(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ActionCapabilities {
    message: bool,
    answer_question: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    permission: Option<PermissionCapabilities>,
    interrupt: bool,
    open_in_terminal: bool,
}

#[derive(Serialize)]
struct PermissionCapabilities {
    approve: bool,
    reject: bool,
}

async fn actions(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
) -> Result<Json<ActionCapabilities>, ApiError> {
    let session = load_session(state, id).await?;
    let has_tmux = session.tmux_info.is_some();
    let has_live_session = session.status != SessionStatus::Ended;
    Ok(Json(ActionCapabilities {
        message: true,
        answer_question: has_live_session
            && session.engine == Engine::Claude
            && is_ask_user_question(&session),
        permission: (has_live_session
            && session.engine == Engine::Claude
            && session.has_pending_permission_requests())
        .then_some(PermissionCapabilities {
            approve: true,
            reject: true,
        }),
        interrupt: has_live_session && has_tmux,
        open_in_terminal: has_tmux,
    }))
}

#[derive(Deserialize)]
struct MessageRequest {
    content: String,
}

async fn send_message(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
    Json(request): Json<MessageRequest>,
) -> Result<Json<Success>, ApiError> {
    let session = load_session(state.clone(), id).await?;
    if request.content.trim().is_empty() {
        return Err(ApiError::bad_request("message must not be empty"));
    }
    let runtime = state.runtime;
    run_blocking(move || runtime.send_message(&session, &request.content)).await?;
    Ok(Json(Success::new()))
}

#[derive(Deserialize)]
struct AnswerRequest {
    choice: u8,
}

async fn answer_question(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
    Json(request): Json<AnswerRequest>,
) -> Result<Json<Success>, ApiError> {
    let session = load_session(state.clone(), id).await?;
    if session.engine != Engine::Claude || !is_ask_user_question(&session) {
        return Err(ApiError::conflict(
            "session is not waiting for a question answer",
        ));
    }
    let tmux_info = require_tmux(&session)?;
    let key = match request.choice {
        1 => "1",
        2 => "2",
        3 => "3",
        4 => "4",
        5 => "5",
        6 => "6",
        7 => "7",
        8 => "8",
        9 => "9",
        _ => return Err(ApiError::bad_request("choice must be between 1 and 9")),
    };
    send_keys(state.runtime, &session, tmux_info, vec![key, "Enter"]).await?;
    Ok(Json(Success::new()))
}

async fn approve_permission(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
) -> Result<Json<Success>, ApiError> {
    resolve_permission_action(state, id, true).await
}

async fn reject_permission(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
) -> Result<Json<Success>, ApiError> {
    resolve_permission_action(state, id, false).await
}

async fn resolve_permission_action(
    state: ActionState,
    id: String,
    approved: bool,
) -> Result<Json<Success>, ApiError> {
    let session = load_session(state.clone(), id).await?;
    if session.engine != Engine::Claude || !session.has_pending_permission_requests() {
        return Err(ApiError::conflict(
            "permission approval is unavailable for this session",
        ));
    }
    let tmux_info = require_tmux(&session)?;
    let keys = if approved {
        vec!["1", "Enter"]
    } else {
        vec!["Escape"]
    };
    send_keys(state.runtime, &session, tmux_info, keys).await?;
    Ok(Json(Success::new()))
}

async fn interrupt(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
) -> Result<Json<Success>, ApiError> {
    let session = load_session(state.clone(), id).await?;
    let tmux_info = require_tmux(&session)?;
    send_keys(state.runtime, &session, tmux_info, vec!["Escape"]).await?;
    Ok(Json(Success::new()))
}

async fn focus(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
) -> Result<Json<Success>, ApiError> {
    let session = load_session(state.clone(), id).await?;
    let tmux_info = require_tmux(&session)?;
    let runtime = state.runtime;
    let tmux_info = tmux_info.clone();
    run_blocking(move || runtime.focus(&tmux_info)).await?;
    Ok(Json(Success::new()))
}

async fn mark_read(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
) -> Result<Json<Success>, ApiError> {
    let session = load_session(state.clone(), id.clone()).await?;
    let sessions_dir = resolve_sessions_dir(state.sessions_dir).map_err(ApiError::internal)?;
    run_blocking(move || {
        store::mark_session_read_in(&sessions_dir, &session.session_id, Utc::now())
    })
    .await?;
    Ok(Json(Success::new()))
}

async fn archive(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
) -> Result<Json<Success>, ApiError> {
    let session = load_session(state.clone(), id.clone()).await?;
    let sessions_dir = resolve_sessions_dir(state.sessions_dir).map_err(ApiError::internal)?;
    let active = run_blocking(move || {
        let mut active = false;
        store::update_session_in(&sessions_dir, &session.session_id, |stored| {
            if stored.archived_at.is_some() {
                return false;
            }
            if is_active_session(stored) {
                active = true;
                return false;
            }
            stored.archived_at = Some(Utc::now());
            true
        })?;
        Ok(active)
    })
    .await?;
    if active {
        return Err(ApiError::conflict("cannot archive an active session"));
    }
    Ok(Json(Success::new()))
}

fn is_active_session(session: &Session) -> bool {
    session.has_active_work(session.status)
        || (session.status != SessionStatus::Ended && session.has_pending_permission_requests())
}

#[derive(Serialize)]
struct Success {
    ok: bool,
}

impl Success {
    fn new() -> Self {
        Self { ok: true }
    }
}

fn require_tmux(session: &Session) -> Result<&TmuxInfo, ApiError> {
    session
        .tmux_info
        .as_ref()
        .ok_or_else(|| ApiError::conflict("session has no tmux pane"))
}

async fn load_session(state: ActionState, id: String) -> Result<Session, ApiError> {
    let session = tokio::task::spawn_blocking(move || {
        let session = match state.sessions_dir {
            Some(dir) => store::load_session_from(&dir, &id),
            None => store::load_session(&id),
        }
        .map_err(|error| {
            if error
                .downcast_ref::<super::super::error::CcError>()
                .is_some_and(|error| {
                    matches!(error, super::super::error::CcError::InvalidSessionId(_))
                })
            {
                ApiError::not_found()
            } else {
                ApiError::internal(error)
            }
        })?;
        Ok::<_, ApiError>(session)
    })
    .await
    .map_err(ApiError::internal)??;
    session
        .filter(|session| session.archived_at.is_none())
        .ok_or_else(ApiError::not_found)
}

fn resolve_sessions_dir(override_dir: Option<PathBuf>) -> Result<PathBuf> {
    match override_dir {
        Some(dir) => Ok(dir),
        None => store::sessions_dir(),
    }
}

async fn send_keys(
    runtime: Arc<dyn ActionRuntime>,
    session: &Session,
    tmux_info: &TmuxInfo,
    keys: Vec<&'static str>,
) -> Result<(), ApiError> {
    if session.status == SessionStatus::Ended {
        return Err(ApiError::conflict("session has ended"));
    }
    let tmux_info = tmux_info.clone();
    let engine = session.engine;
    let result = tokio::task::spawn_blocking(move || {
        let Some(current_command) = runtime.pane_current_command(&tmux_info) else {
            return Ok(false);
        };
        if SHELL_COMMANDS.contains(&current_command.as_str()) {
            return Ok(false);
        }
        runtime.send_keys_if_agent_is_running(&tmux_info, engine, &current_command, &keys)
    })
    .await
    .map_err(ApiError::internal)?;
    match result {
        Ok(true) => Ok(()),
        Ok(false) => Err(ApiError::conflict("session pane is idle or unavailable")),
        Err(error) => Err(ApiError::operation(error)),
    }
}

async fn run_blocking<T>(
    operation: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T, ApiError>
where
    T: Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::operation)
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        Self::operation(error)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;
    use std::sync::Mutex;

    use axum::body::{Body, to_bytes};
    use axum::http::{Request, Response, StatusCode};
    use chrono::TimeZone;
    use rstest::{fixture, rstest};
    use serde_json::Value;
    use tempfile::TempDir;
    use tower::ServiceExt;

    use super::*;
    use crate::commands::agent::types::TmuxInfo;

    struct FakeRuntime {
        calls: Mutex<Vec<Value>>,
        pane_command: Mutex<Option<String>>,
        pane_has_agent_process: Mutex<bool>,
        pane_command_matches: Mutex<bool>,
    }

    impl Default for FakeRuntime {
        fn default() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                pane_command: Mutex::new(Some("claude".to_string())),
                pane_has_agent_process: Mutex::new(true),
                pane_command_matches: Mutex::new(true),
            }
        }
    }

    impl ActionRuntime for FakeRuntime {
        fn send_message(&self, session: &Session, content: &str) -> Result<()> {
            self.calls.lock().unwrap().push(json!({
                "kind": "message",
                "sessionId": session.session_id,
                "content": content,
            }));
            Ok(())
        }

        fn pane_current_command(&self, tmux_info: &TmuxInfo) -> Option<String> {
            self.calls.lock().unwrap().push(json!({
                "kind": "pane-command",
                "paneId": tmux_info.pane_id,
                "socketPath": tmux_info.socket_path,
            }));
            self.pane_command.lock().unwrap().clone()
        }

        fn send_keys_if_agent_is_running(
            &self,
            tmux_info: &TmuxInfo,
            engine: Engine,
            current_command: &str,
            keys: &[&str],
        ) -> Result<bool> {
            let pane_has_agent_process = *self.pane_has_agent_process.lock().unwrap();
            let pane_command_matches = *self.pane_command_matches.lock().unwrap();
            self.calls.lock().unwrap().push(json!({
                "kind": "guarded-keys",
                "paneId": tmux_info.pane_id,
                "engine": engine,
                "currentCommand": current_command,
                "keys": &keys,
                "paneHasAgentProcess": pane_has_agent_process,
                "paneCommandMatches": pane_command_matches,
            }));
            if !pane_has_agent_process || !pane_command_matches {
                return Ok(false);
            }
            self.calls.lock().unwrap().push(json!({
                "kind": "keys",
                "paneId": tmux_info.pane_id,
                "socketPath": tmux_info.socket_path,
                "keys": keys,
            }));
            Ok(true)
        }

        fn focus(&self, tmux_info: &TmuxInfo) -> Result<()> {
            self.calls.lock().unwrap().push(json!({
                "kind": "focus",
                "paneId": tmux_info.pane_id,
                "socketPath": tmux_info.socket_path,
            }));
            Ok(())
        }
    }

    struct ActionContext {
        temp_dir: TempDir,
        runtime: Arc<FakeRuntime>,
    }

    impl ActionContext {
        fn router(&self) -> Router {
            router_with_state(ActionState {
                sessions_dir: Some(self.temp_dir.path().join("sessions")),
                runtime: self.runtime.clone(),
            })
        }

        fn save_session(&self, session: &Session) {
            store::save_session_to(&self.temp_dir.path().join("sessions"), session).unwrap();
        }

        fn load_session(&self, session_id: &str) -> Session {
            store::load_session_from(&self.temp_dir.path().join("sessions"), session_id)
                .unwrap()
                .unwrap()
        }

        fn set_pane_command(&self, command: Option<&str>) {
            *self.runtime.pane_command.lock().unwrap() = command.map(str::to_string);
        }

        fn set_pane_has_agent_process(&self, has_agent_process: bool) {
            *self.runtime.pane_has_agent_process.lock().unwrap() = has_agent_process;
        }

        fn set_pane_command_matches(&self, matches: bool) {
            *self.runtime.pane_command_matches.lock().unwrap() = matches;
        }
    }

    #[fixture]
    fn action_context() -> ActionContext {
        ActionContext {
            temp_dir: TempDir::new().unwrap(),
            runtime: Arc::new(FakeRuntime::default()),
        }
    }

    fn session(session_id: &str) -> Session {
        Session {
            session_id: session_id.to_string(),
            archived_at: None,
            pr_context: None,
            crit_urls: Vec::new(),
            pending_human_review_ids: BTreeSet::new(),
            cwd: PathBuf::from("/tmp/example"),
            transcript_path: None,
            tty: None,
            tmux_info: Some(TmuxInfo {
                session_name: "fixture".to_string(),
                window_name: "fixture".to_string(),
                window_index: 0,
                pane_id: "%4".to_string(),
                socket_path: Some("/tmp/example-tmux.sock".to_string()),
            }),
            status: SessionStatus::WaitingInput,
            agent_status: None,
            agent_status_clear_pending: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
            label: None,
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            child_launch: Default::default(),
            ancestor_session_ids: Vec::new(),
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

    async fn response_snapshot(response: Response<Body>, context: &ActionContext) -> Value {
        let status = response.status().as_u16();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        json!({
            "status": status,
            "body": serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null),
            "calls": context.runtime.calls.lock().unwrap().clone(),
        })
    }

    fn post(uri: &str, body: Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap()
    }

    fn post_empty(uri: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .body(Body::empty())
            .unwrap()
    }

    fn normalize_session(mut session: Session) -> Value {
        let timestamp = Utc.timestamp_opt(1, 0).single().unwrap();
        if session.read_at.is_some() {
            session.read_at = Some(timestamp);
        }
        if session.archived_at.is_some() {
            session.archived_at = Some(timestamp);
        }
        serde_json::to_value(session).unwrap()
    }

    #[rstest]
    #[tokio::test]
    async fn send_message_forwards_nonempty_content_unchanged(action_context: ActionContext) {
        action_context.save_session(&session("sample-session"));

        let response = action_context
            .router()
            .oneshot(post(
                "/sessions/sample-session/messages",
                json!({ "content": "  continue the investigation  " }),
            ))
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response, &action_context).await,
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [{
                    "kind": "message",
                    "sessionId": "sample-session",
                    "content": "  continue the investigation  ",
                }],
            }),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn send_message_rejects_whitespace_without_calling_runtime(
        action_context: ActionContext,
    ) {
        action_context.save_session(&session("sample-session"));

        let response = action_context
            .router()
            .oneshot(post(
                "/sessions/sample-session/messages",
                json!({ "content": "  \n " }),
            ))
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response, &action_context).await,
            json!({
                "status": StatusCode::BAD_REQUEST.as_u16(),
                "body": { "error": "message must not be empty" },
                "calls": [],
            }),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn answer_question_sends_only_the_bounded_server_key(action_context: ActionContext) {
        let mut session = session("sample-session");
        session.current_tool = Some("AskUserQuestion".to_string());
        action_context.save_session(&session);

        let response = action_context
            .router()
            .oneshot(post(
                "/sessions/sample-session/answer",
                json!({ "choice": 2 }),
            ))
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response, &action_context).await,
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [{
                    "kind": "pane-command",
                    "paneId": "%4",
                    "socketPath": "/tmp/example-tmux.sock",
                }, {
                    "kind": "guarded-keys",
                    "paneId": "%4",
                    "engine": "claude",
                    "currentCommand": "claude",
                    "keys": ["2", "Enter"],
                    "paneHasAgentProcess": true,
                    "paneCommandMatches": true,
                }, {
                    "kind": "keys",
                    "paneId": "%4",
                    "socketPath": "/tmp/example-tmux.sock",
                    "keys": ["2", "Enter"],
                }],
            }),
        );
    }

    #[rstest]
    #[case::approve(true, vec!["1", "Enter"])]
    #[case::reject(false, vec!["Escape"])]
    #[tokio::test]
    async fn permission_actions_send_fixed_keys(
        #[case] approved: bool,
        #[case] expected_keys: Vec<&str>,
        action_context: ActionContext,
    ) {
        let mut session = session("sample-session");
        session
            .pending_permission_agent_ids
            .insert("main".to_string());
        action_context.save_session(&session);
        let action = if approved { "approve" } else { "reject" };

        let response = action_context
            .router()
            .oneshot(post_empty(&format!(
                "/sessions/sample-session/permission/{action}"
            )))
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response, &action_context).await,
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [{
                    "kind": "pane-command",
                    "paneId": "%4",
                    "socketPath": "/tmp/example-tmux.sock",
                }, {
                    "kind": "guarded-keys",
                    "paneId": "%4",
                    "engine": "claude",
                    "currentCommand": "claude",
                    "keys": expected_keys,
                    "paneHasAgentProcess": true,
                    "paneCommandMatches": true,
                }, {
                    "kind": "keys",
                    "paneId": "%4",
                    "socketPath": "/tmp/example-tmux.sock",
                    "keys": expected_keys,
                }],
            }),
        );
    }

    #[rstest]
    #[case::zero(0)]
    #[case::above_supported_choices(10)]
    #[tokio::test]
    async fn answer_question_rejects_out_of_range_choices(
        #[case] choice: u8,
        action_context: ActionContext,
    ) {
        let mut session = session("sample-session");
        session.current_tool = Some("AskUserQuestion".to_string());
        action_context.save_session(&session);

        let response = action_context
            .router()
            .oneshot(post(
                "/sessions/sample-session/answer",
                json!({ "choice": choice }),
            ))
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response, &action_context).await,
            json!({
                "status": StatusCode::BAD_REQUEST.as_u16(),
                "body": { "error": "choice must be between 1 and 9" },
                "calls": [],
            }),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn codex_permission_capability_is_disabled_and_approval_is_rejected(
        action_context: ActionContext,
    ) {
        let mut session = session("sample-session");
        session.engine = Engine::Codex;
        session.current_tool = Some("AskUserQuestion".to_string());
        session
            .pending_permission_agent_ids
            .insert("main".to_string());
        action_context.save_session(&session);
        let app = action_context.router();

        let capabilities = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/sessions/sample-session/actions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let approval = app
            .clone()
            .oneshot(post(
                "/sessions/sample-session/permission/approve",
                json!({}),
            ))
            .await
            .unwrap();
        let answer = app
            .oneshot(post(
                "/sessions/sample-session/answer",
                json!({ "choice": 1 }),
            ))
            .await
            .unwrap();

        assert_eq!(
            (
                response_snapshot(capabilities, &action_context).await,
                response_snapshot(approval, &action_context).await,
                response_snapshot(answer, &action_context).await,
            ),
            (
                json!({
                    "status": StatusCode::OK.as_u16(),
                    "body": {
                        "message": true,
                        "answerQuestion": false,
                        "interrupt": true,
                        "openInTerminal": true,
                    },
                    "calls": [],
                }),
                json!({
                    "status": StatusCode::CONFLICT.as_u16(),
                    "body": { "error": "permission approval is unavailable for this session" },
                    "calls": [],
                }),
                json!({
                    "status": StatusCode::CONFLICT.as_u16(),
                    "body": { "error": "session is not waiting for a question answer" },
                    "calls": [],
                }),
            ),
        );
    }

    #[rstest]
    #[case::not_registered("not-stored")]
    #[case::invalid_id("bad..id")]
    #[tokio::test]
    async fn unknown_session_is_not_found_without_calling_runtime(
        #[case] id: &str,
        action_context: ActionContext,
    ) {
        let response = action_context
            .router()
            .oneshot(post(
                &format!("/sessions/{id}/messages"),
                json!({ "content": "hello" }),
            ))
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response, &action_context).await,
            json!({
                "status": StatusCode::NOT_FOUND.as_u16(),
                "body": { "error": "session not found" },
                "calls": [],
            }),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn mark_read_persists_read_at_for_a_stopped_session(action_context: ActionContext) {
        let mut session = session("sample-session");
        session.status = SessionStatus::Stopped;
        session.tmux_info = None;
        action_context.save_session(&session);

        let response = action_context
            .router()
            .oneshot(post_empty("/sessions/sample-session/read"))
            .await
            .unwrap();
        let stored = action_context.load_session("sample-session");
        let mut expected = session;
        expected.read_at = Some(Utc::now());

        assert_eq!(
            json!({
                "response": response_snapshot(response, &action_context).await,
                "session": normalize_session(stored),
            }),
            json!({
                "response": {
                    "status": StatusCode::OK.as_u16(),
                    "body": { "ok": true },
                    "calls": [],
                },
                "session": normalize_session(expected),
            }),
        );
    }

    #[rstest]
    #[case::claude(Engine::Claude, "claude")]
    #[case::codex(Engine::Codex, "a")]
    #[tokio::test]
    async fn interrupt_sends_escape_to_the_session_pane(
        #[case] engine: Engine,
        #[case] current_command: &str,
        action_context: ActionContext,
    ) {
        let mut session = session("sample-session");
        session.engine = engine;
        action_context.save_session(&session);
        action_context.set_pane_command(Some(current_command));

        let response = action_context
            .router()
            .oneshot(post_empty("/sessions/sample-session/interrupt"))
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response, &action_context).await,
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [{
                    "kind": "pane-command",
                    "paneId": "%4",
                    "socketPath": "/tmp/example-tmux.sock",
                }, {
                    "kind": "guarded-keys",
                    "paneId": "%4",
                    "engine": engine,
                    "currentCommand": current_command,
                    "keys": ["Escape"],
                    "paneHasAgentProcess": true,
                    "paneCommandMatches": true,
                }, {
                    "kind": "keys",
                    "paneId": "%4",
                    "socketPath": "/tmp/example-tmux.sock",
                    "keys": ["Escape"],
                }],
            }),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn focus_uses_the_session_pane_and_recorded_socket(action_context: ActionContext) {
        action_context.save_session(&session("sample-session"));

        let response = action_context
            .router()
            .oneshot(post_empty("/sessions/sample-session/focus"))
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response, &action_context).await,
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [{
                    "kind": "focus",
                    "paneId": "%4",
                    "socketPath": "/tmp/example-tmux.sock",
                }],
            }),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn archive_hides_the_session_from_later_actions_lookups(action_context: ActionContext) {
        let mut session = session("sample-session");
        session.status = SessionStatus::Stopped;
        action_context.save_session(&session);
        let app = action_context.router();

        let archived = app
            .clone()
            .oneshot(post_empty("/sessions/sample-session/archive"))
            .await
            .unwrap();
        let lookup = app
            .oneshot(
                Request::builder()
                    .uri("/sessions/sample-session/actions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            (
                response_snapshot(archived, &action_context).await,
                response_snapshot(lookup, &action_context).await,
                normalize_session(action_context.load_session("sample-session")),
            ),
            (
                json!({ "status": StatusCode::OK.as_u16(), "body": { "ok": true }, "calls": [] }),
                json!({ "status": StatusCode::NOT_FOUND.as_u16(), "body": { "error": "session not found" }, "calls": [] }),
                normalize_session({
                    let mut expected = session;
                    expected.archived_at = Some(Utc::now());
                    expected
                }),
            ),
        );
    }

    #[rstest]
    #[case::answer_missing("answer", None, true, true)]
    #[case::answer_shell("answer", Some("zsh"), true, true)]
    #[case::answer_other_program("answer", Some("nvim"), false, true)]
    #[case::permission_missing("permission", None, true, true)]
    #[case::permission_shell("permission", Some("bash"), true, true)]
    #[case::permission_other_program("permission", Some("nvim"), false, true)]
    #[case::interrupt_missing("interrupt", None, true, true)]
    #[case::interrupt_shell("interrupt", Some("fish"), true, true)]
    #[case::interrupt_other_program("interrupt", Some("nvim"), false, true)]
    #[case::interrupt_command_changed("interrupt", Some("claude"), true, false)]
    #[tokio::test]
    async fn key_actions_reject_unavailable_or_idle_panes(
        #[case] action: &str,
        #[case] pane_command: Option<&str>,
        #[case] pane_has_agent_process: bool,
        #[case] pane_command_matches: bool,
        action_context: ActionContext,
    ) {
        let mut session = session("sample-session");
        let (uri, body, keys) = match action {
            "answer" => {
                session.current_tool = Some("AskUserQuestion".to_string());
                (
                    "/sessions/sample-session/answer",
                    json!({ "choice": 1 }),
                    vec!["1", "Enter"],
                )
            }
            "permission" => {
                session
                    .pending_permission_agent_ids
                    .insert("main".to_string());
                (
                    "/sessions/sample-session/permission/approve",
                    json!({}),
                    vec!["1", "Enter"],
                )
            }
            "interrupt" => (
                "/sessions/sample-session/interrupt",
                json!({}),
                vec!["Escape"],
            ),
            _ => unreachable!(),
        };
        action_context.save_session(&session);
        action_context.set_pane_command(pane_command);
        action_context.set_pane_has_agent_process(pane_has_agent_process);
        action_context.set_pane_command_matches(pane_command_matches);

        let response = action_context
            .router()
            .oneshot(post(uri, body))
            .await
            .unwrap();

        let mut expected_calls = vec![json!({
            "kind": "pane-command",
            "paneId": "%4",
            "socketPath": "/tmp/example-tmux.sock",
        })];
        if pane_command.is_some_and(|command| !SHELL_COMMANDS.contains(&command)) {
            expected_calls.push(json!({
                "kind": "guarded-keys",
                "paneId": "%4",
                "engine": "claude",
                "currentCommand": pane_command,
                "keys": &keys,
                "paneHasAgentProcess": pane_has_agent_process,
                "paneCommandMatches": pane_command_matches,
            }));
            if pane_has_agent_process && pane_command_matches {
                expected_calls.push(json!({
                    "kind": "keys",
                    "paneId": "%4",
                    "socketPath": "/tmp/example-tmux.sock",
                    "keys": &keys,
                }));
            }
        }

        assert_eq!(
            response_snapshot(response, &action_context).await,
            json!({
                "status": StatusCode::CONFLICT.as_u16(),
                "body": { "error": "session pane is idle or unavailable" },
                "calls": expected_calls,
            }),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn ended_sessions_hide_and_reject_key_actions(action_context: ActionContext) {
        let mut session = session("sample-session");
        session.status = SessionStatus::Ended;
        session.current_tool = Some("AskUserQuestion".to_string());
        session
            .pending_permission_agent_ids
            .insert("main".to_string());
        action_context.save_session(&session);
        let app = action_context.router();

        let capabilities = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/sessions/sample-session/actions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let answer = app
            .clone()
            .oneshot(post(
                "/sessions/sample-session/answer",
                json!({ "choice": 1 }),
            ))
            .await
            .unwrap();
        let permission = app
            .clone()
            .oneshot(post_empty("/sessions/sample-session/permission/approve"))
            .await
            .unwrap();
        let interrupt = app
            .oneshot(post_empty("/sessions/sample-session/interrupt"))
            .await
            .unwrap();

        assert_eq!(
            (
                response_snapshot(capabilities, &action_context).await,
                response_snapshot(answer, &action_context).await,
                response_snapshot(permission, &action_context).await,
                response_snapshot(interrupt, &action_context).await,
            ),
            (
                json!({
                    "status": StatusCode::OK.as_u16(),
                    "body": {
                        "message": true,
                        "answerQuestion": false,
                        "interrupt": false,
                        "openInTerminal": true,
                    },
                    "calls": [],
                }),
                json!({
                    "status": StatusCode::CONFLICT.as_u16(),
                    "body": { "error": "session has ended" },
                    "calls": [],
                }),
                json!({
                    "status": StatusCode::CONFLICT.as_u16(),
                    "body": { "error": "session has ended" },
                    "calls": [],
                }),
                json!({
                    "status": StatusCode::CONFLICT.as_u16(),
                    "body": { "error": "session has ended" },
                    "calls": [],
                }),
            ),
        );
    }

    #[rstest]
    #[case::running(SessionStatus::Running, false, false)]
    #[case::waiting_for_input(SessionStatus::WaitingInput, false, false)]
    #[case::failed_with_pending_permission(SessionStatus::Failed, true, false)]
    #[case::stopped_with_pending_task(SessionStatus::Stopped, false, true)]
    #[tokio::test]
    async fn archive_rejects_active_sessions(
        #[case] status: SessionStatus,
        #[case] pending_permission: bool,
        #[case] pending_task: bool,
        action_context: ActionContext,
    ) {
        let mut session = session("sample-session");
        session.status = status;
        if pending_permission {
            session
                .pending_permission_agent_ids
                .insert("subagent".to_string());
        }
        if pending_task {
            session.pending_bg_task_ids.insert("task".to_string());
        }
        action_context.save_session(&session);

        let response = action_context
            .router()
            .oneshot(post_empty("/sessions/sample-session/archive"))
            .await
            .unwrap();

        assert_eq!(
            json!({
                "response": response_snapshot(response, &action_context).await,
                "session": normalize_session(action_context.load_session("sample-session")),
            }),
            json!({
                "response": {
                    "status": StatusCode::CONFLICT.as_u16(),
                    "body": { "error": "cannot archive an active session" },
                    "calls": [],
                },
                "session": normalize_session(session),
            }),
        );
    }
}
