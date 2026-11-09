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
use super::super::store;
use super::super::types::{Engine, Session, TmuxInfo};

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
    fn send_keys(&self, tmux_info: &TmuxInfo, keys: &[&str]) -> Result<()>;
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

    fn send_keys(&self, tmux_info: &TmuxInfo, keys: &[&str]) -> Result<()> {
        for key in keys {
            crate::infra::tmux::send_keys_for_socket(
                tmux_info.socket_path.as_deref(),
                &tmux_info.pane_id,
                key,
            )?;
        }
        Ok(())
    }

    fn focus(&self, tmux_info: &TmuxInfo) -> Result<()> {
        crate::infra::tmux::focus_pane(&tmux_info.pane_id).map_err(anyhow::Error::from)
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
    Ok(Json(ActionCapabilities {
        message: true,
        answer_question: is_ask_user_question(&session),
        permission: (session.engine == Engine::Claude && session.has_pending_permission_requests())
            .then_some(PermissionCapabilities {
                approve: true,
                reject: true,
            }),
        interrupt: has_tmux,
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
    if !(1..=9).contains(&request.choice) {
        return Err(ApiError::bad_request("choice must be between 1 and 9"));
    }
    if !is_ask_user_question(&session) {
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
    send_keys(state.runtime, tmux_info, vec![key, "Enter"]).await?;
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
        vec!["3", "Enter"]
    };
    send_keys(state.runtime, tmux_info, keys).await?;
    Ok(Json(Success::new()))
}

async fn interrupt(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
) -> Result<Json<Success>, ApiError> {
    let session = load_session(state.clone(), id).await?;
    let tmux_info = require_tmux(&session)?;
    send_keys(state.runtime, tmux_info, vec!["Escape"]).await?;
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
    run_blocking(move || {
        store::update_session_in(&sessions_dir, &session.session_id, |stored| {
            if stored.archived_at.is_some() {
                return false;
            }
            stored.archived_at = Some(Utc::now());
            true
        })
    })
    .await?;
    Ok(Json(Success::new()))
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

fn is_ask_user_question(session: &Session) -> bool {
    session
        .current_tool
        .as_deref()
        .is_some_and(|tool| tool == "AskUserQuestion" || tool.starts_with("AskUserQuestion("))
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
    tmux_info: &TmuxInfo,
    keys: Vec<&'static str>,
) -> Result<(), ApiError> {
    let tmux_info = tmux_info.clone();
    run_blocking(move || runtime.send_keys(&tmux_info, &keys)).await
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
    use serde_json::Value;
    use tempfile::TempDir;
    use tower::ServiceExt;

    use super::*;
    use crate::commands::agent::types::{SessionStatus, TmuxInfo};

    #[derive(Default)]
    struct FakeRuntime {
        calls: Mutex<Vec<Value>>,
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

        fn send_keys(&self, tmux_info: &TmuxInfo, keys: &[&str]) -> Result<()> {
            self.calls.lock().unwrap().push(json!({
                "kind": "keys",
                "paneId": tmux_info.pane_id,
                "keys": keys,
            }));
            Ok(())
        }

        fn focus(&self, tmux_info: &TmuxInfo) -> Result<()> {
            self.calls.lock().unwrap().push(json!({
                "kind": "focus",
                "paneId": tmux_info.pane_id,
            }));
            Ok(())
        }
    }

    fn test_router(temp_dir: &TempDir, runtime: Arc<FakeRuntime>) -> Router {
        let sessions_dir = temp_dir.path().join("sessions");
        router_with_state(ActionState {
            sessions_dir: Some(sessions_dir),
            runtime,
        })
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
                socket_path: None,
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

    async fn response_snapshot(response: Response<Body>, calls: &Mutex<Vec<Value>>) -> Value {
        let status = response.status().as_u16();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        json!({
            "status": status,
            "body": serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null),
            "calls": calls.lock().unwrap().clone(),
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

    #[tokio::test]
    async fn answer_question_sends_only_the_bounded_server_key() {
        let temp_dir = TempDir::new().unwrap();
        let runtime = Arc::new(FakeRuntime::default());
        let mut session = session("sample-session");
        session.current_tool = Some("AskUserQuestion".to_string());
        store::save_session_to(&temp_dir.path().join("sessions"), &session).unwrap();

        let response = test_router(&temp_dir, runtime.clone())
            .oneshot(post(
                "/sessions/sample-session/answer",
                json!({ "choice": 2 }),
            ))
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response, &runtime.calls).await,
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [{ "kind": "keys", "paneId": "%4", "keys": ["2", "Enter"] }],
            }),
        );
    }

    #[rstest::rstest]
    #[case::approve(true, ["1", "Enter"])]
    #[case::reject(false, ["3", "Enter"])]
    #[tokio::test]
    async fn permission_actions_send_fixed_keys(
        #[case] approved: bool,
        #[case] expected_keys: [&str; 2],
    ) {
        let temp_dir = TempDir::new().unwrap();
        let runtime = Arc::new(FakeRuntime::default());
        let mut session = session("sample-session");
        session
            .pending_permission_agent_ids
            .insert("main".to_string());
        store::save_session_to(&temp_dir.path().join("sessions"), &session).unwrap();
        let action = if approved { "approve" } else { "reject" };

        let response = test_router(&temp_dir, runtime.clone())
            .oneshot(post_empty(&format!(
                "/sessions/sample-session/permission/{action}"
            )))
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response, &runtime.calls).await,
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [{ "kind": "keys", "paneId": "%4", "keys": expected_keys }],
            }),
        );
    }

    #[rstest::rstest]
    #[case::zero(0)]
    #[case::above_supported_choices(10)]
    #[tokio::test]
    async fn answer_question_rejects_out_of_range_choices(#[case] choice: u8) {
        let temp_dir = TempDir::new().unwrap();
        let runtime = Arc::new(FakeRuntime::default());
        let mut session = session("sample-session");
        session.current_tool = Some("AskUserQuestion".to_string());
        store::save_session_to(&temp_dir.path().join("sessions"), &session).unwrap();

        let response = test_router(&temp_dir, runtime.clone())
            .oneshot(post(
                "/sessions/sample-session/answer",
                json!({ "choice": choice }),
            ))
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response, &runtime.calls).await,
            json!({
                "status": StatusCode::BAD_REQUEST.as_u16(),
                "body": { "error": "choice must be between 1 and 9" },
                "calls": [],
            }),
        );
    }

    #[tokio::test]
    async fn codex_permission_capability_is_disabled_and_approval_is_rejected() {
        let temp_dir = TempDir::new().unwrap();
        let runtime = Arc::new(FakeRuntime::default());
        let mut session = session("sample-session");
        session.engine = Engine::Codex;
        session
            .pending_permission_agent_ids
            .insert("main".to_string());
        store::save_session_to(&temp_dir.path().join("sessions"), &session).unwrap();
        let app = test_router(&temp_dir, runtime.clone());

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
            .oneshot(post(
                "/sessions/sample-session/permission/approve",
                json!({}),
            ))
            .await
            .unwrap();

        assert_eq!(
            (
                response_snapshot(capabilities, &runtime.calls).await,
                response_snapshot(approval, &runtime.calls).await,
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
            ),
        );
    }

    #[rstest::rstest]
    #[case::not_registered("not-stored")]
    #[case::invalid_id("bad..id")]
    #[tokio::test]
    async fn unknown_session_is_not_found_without_calling_runtime(#[case] id: &str) {
        let temp_dir = TempDir::new().unwrap();
        let runtime = Arc::new(FakeRuntime::default());
        let response = test_router(&temp_dir, runtime.clone())
            .oneshot(post(
                &format!("/sessions/{id}/messages"),
                json!({ "content": "hello" }),
            ))
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response, &runtime.calls).await,
            json!({
                "status": StatusCode::NOT_FOUND.as_u16(),
                "body": { "error": "session not found" },
                "calls": [],
            }),
        );
    }

    #[tokio::test]
    async fn mark_read_persists_read_at_for_a_stopped_session() {
        let temp_dir = TempDir::new().unwrap();
        let runtime = Arc::new(FakeRuntime::default());
        let mut session = session("sample-session");
        session.status = SessionStatus::Stopped;
        session.tmux_info = None;
        store::save_session_to(&temp_dir.path().join("sessions"), &session).unwrap();

        let response = test_router(&temp_dir, runtime.clone())
            .oneshot(post_empty("/sessions/sample-session/read"))
            .await
            .unwrap();
        let stored = store::load_session_from(&temp_dir.path().join("sessions"), "sample-session")
            .unwrap()
            .unwrap();

        assert_eq!(
            (
                response_snapshot(response, &runtime.calls).await,
                stored.read_at.is_some(),
            ),
            (
                json!({ "status": StatusCode::OK.as_u16(), "body": { "ok": true }, "calls": [] }),
                true,
            ),
        );
    }

    #[tokio::test]
    async fn archive_hides_the_session_from_later_actions_lookups() {
        let temp_dir = TempDir::new().unwrap();
        let runtime = Arc::new(FakeRuntime::default());
        let session = session("sample-session");
        store::save_session_to(&temp_dir.path().join("sessions"), &session).unwrap();
        let app = test_router(&temp_dir, runtime.clone());

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
        let stored = store::load_session_from(&temp_dir.path().join("sessions"), "sample-session")
            .unwrap()
            .unwrap();

        assert_eq!(
            (
                response_snapshot(archived, &runtime.calls).await,
                response_snapshot(lookup, &runtime.calls).await,
                stored.archived_at.is_some(),
            ),
            (
                json!({ "status": StatusCode::OK.as_u16(), "body": { "ok": true }, "calls": [] }),
                json!({ "status": StatusCode::NOT_FOUND.as_u16(), "body": { "error": "session not found" }, "calls": [] }),
                true,
            ),
        );
    }
}
