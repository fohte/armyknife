use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use axum::extract::{DefaultBodyLimit, Path as RoutePath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::super::resume::SHELL_COMMANDS;
use super::super::store;
use super::super::store::PermissionRequestDecision;
use super::super::types::{Engine, MAIN_THREAD_AGENT_KEY, Session, SessionStatus, TmuxInfo};
use super::super::user_turn::is_ask_user_question;
use super::messages::RouteLoaders;

mod decisions;
mod messages;
mod proposal_answer_locks;
mod proposals;

use proposal_answer_locks::ProposalAnswerLocks;

pub(super) fn router(message_loaders: RouteLoaders) -> Router {
    router_with_state(ActionState {
        sessions_dir: None,
        attachment_dir: None,
        runtime: Arc::new(LiveActionRuntime),
        proposal_answer_locks: ProposalAnswerLocks::default(),
        message_loaders,
    })
}

fn router_with_state(state: ActionState) -> Router {
    Router::new()
        .route("/sessions/{id}/actions", get(actions))
        .route(
            "/sessions/{id}/messages",
            post(messages::send_message)
                .layer(DefaultBodyLimit::max(messages::MAX_MESSAGE_BODY_BYTES)),
        )
        .route(
            "/sessions/{id}/decisions/{seq}/ok",
            post(decisions::mark_ok),
        )
        .route(
            "/sessions/{id}/decisions/{seq}/change",
            post(decisions::request_change),
        )
        .route(
            "/sessions/{id}/proposals/{seq}/answer",
            post(proposals::answer_proposal),
        )
        .route("/sessions/{id}/answer", post(answer_question))
        .route(
            "/sessions/{id}/permission/approve",
            post(approve_permission),
        )
        .route("/sessions/{id}/permission/reject", post(reject_permission))
        .route("/sessions/{id}/interrupt", post(interrupt))
        .route("/sessions/{id}/focus", post(focus))
        .route("/sessions/{id}/read", post(mark_read))
        .route("/sessions/{id}/primary-task", post(set_primary_task))
        .route("/sessions/{id}/archive", post(archive))
        .with_state(state)
}

#[derive(Clone)]
struct ActionState {
    sessions_dir: Option<PathBuf>,
    attachment_dir: Option<PathBuf>,
    runtime: Arc<dyn ActionRuntime>,
    message_loaders: RouteLoaders,
    // Delivery precedes persistence, so repeated answers for one proposal must serialize.
    proposal_answer_locks: ProposalAnswerLocks,
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
        messages::send_live_message(session, content)
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

    fn decision_not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: "decision not found".to_string(),
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
        message: has_live_session,
        answer_question: has_live_session
            && session.engine == Engine::Claude
            && pending_main_permission_request_id(&session).is_some()
            && is_ask_user_question(&session),
        permission: (has_live_session
            && session.engine == Engine::Claude
            && pending_main_permission_request_id(&session).is_some())
        .then_some(PermissionCapabilities {
            approve: true,
            reject: true,
        }),
        interrupt: has_live_session && has_tmux,
        open_in_terminal: has_tmux,
    }))
}

#[derive(Deserialize)]
struct AnswerRequest {
    question: Option<String>,
    choice: Option<u8>,
    answer: Option<String>,
    #[serde(default)]
    skip: bool,
}

enum QuestionResponse<'a> {
    Choice(u8),
    Text(&'a str),
}

async fn answer_question(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
    Json(request): Json<AnswerRequest>,
) -> Result<Json<Success>, ApiError> {
    let session = load_session(state.clone(), id).await?;
    if session.status == SessionStatus::Ended {
        return Err(ApiError::conflict("session has ended"));
    }
    if session.engine != Engine::Claude || !is_ask_user_question(&session) {
        return Err(ApiError::conflict(
            "session is not waiting for a question answer",
        ));
    }
    let Some(request_id) = pending_main_permission_request_id(&session) else {
        return Err(ApiError::conflict(
            "session is not waiting for a question answer",
        ));
    };
    let Some(question) = request.question.as_deref() else {
        return Err(ApiError::bad_request("question is required"));
    };
    let sessions_dir = resolve_sessions_dir(state.sessions_dir).map_err(ApiError::internal)?;
    let request_data = run_blocking({
        let sessions_dir = sessions_dir.clone();
        let session_id = session.session_id.clone();
        let request_id = request_id.to_string();
        move || store::permission_request_data(&sessions_dir, &session_id, &request_id)
    })
    .await?
    .ok_or_else(|| ApiError::conflict("question request is no longer pending"))?;
    let decision = if request.skip {
        if request.choice.is_some() || request.answer.is_some() {
            return Err(ApiError::bad_request(
                "an answer cannot be provided when skipping a question",
            ));
        }
        PermissionRequestDecision::Deny {
            message: "Question skipped from watch-web.".to_string(),
            interrupt: false,
        }
    } else {
        let response = match (request.choice, request.answer.as_deref()) {
            (Some(_), Some(_)) => {
                return Err(ApiError::bad_request(
                    "choice and answer cannot both be provided",
                ));
            }
            (Some(choice), None) => QuestionResponse::Choice(choice),
            (None, Some(answer)) => QuestionResponse::Text(answer),
            (None, None) => {
                return Err(ApiError::bad_request(
                    "choice or answer is required unless skipping a question",
                ));
            }
        };
        question_answer_decision(&request_data, question, response)?
    };
    if request_data.tool_name.as_deref() != Some("AskUserQuestion")
        || !request_data
            .tool_input
            .get("questions")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|questions| {
                questions.iter().any(|item| {
                    item.get("question").and_then(serde_json::Value::as_str) == Some(question)
                })
            })
    {
        return Err(ApiError::conflict("question request is no longer pending"));
    }
    let session_id = session.session_id.clone();
    let request_id = request_id.to_string();
    let resolved = run_blocking(move || {
        store::resolve_permission_request(
            &sessions_dir,
            &session_id,
            MAIN_THREAD_AGENT_KEY,
            &request_id,
            &decision,
        )
    })
    .await?;
    if !resolved {
        return Err(ApiError::conflict("question request is no longer pending"));
    }
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
    if session.status == SessionStatus::Ended {
        return Err(ApiError::conflict("session has ended"));
    }
    let Some(request_id) = pending_main_permission_request_id(&session) else {
        return Err(ApiError::conflict(
            "permission approval is unavailable for this session",
        ));
    };
    if session.engine != Engine::Claude {
        return Err(ApiError::conflict(
            "permission approval is unavailable for this session",
        ));
    }
    let decision = if approved {
        PermissionRequestDecision::Allow {
            updated_input: None,
        }
    } else {
        PermissionRequestDecision::Deny {
            message: "Permission request rejected from watch-web.".to_string(),
            interrupt: false,
        }
    };
    let sessions_dir = resolve_sessions_dir(state.sessions_dir).map_err(ApiError::internal)?;
    let session_id = session.session_id.clone();
    let request_id = request_id.to_string();
    let resolved = run_blocking(move || {
        store::resolve_permission_request(
            &sessions_dir,
            &session_id,
            MAIN_THREAD_AGENT_KEY,
            &request_id,
            &decision,
        )
    })
    .await?;
    if !resolved {
        return Err(ApiError::conflict(
            "permission approval is unavailable for this session",
        ));
    }
    Ok(Json(Success::new()))
}

fn pending_main_permission_request_id(session: &Session) -> Option<&str> {
    session
        .pending_permission_agent_ids
        .contains(MAIN_THREAD_AGENT_KEY)
        .then(|| {
            session
                .pending_permission_request_ids
                .get(MAIN_THREAD_AGENT_KEY)
        })
        .flatten()
        .map(String::as_str)
}

fn question_answer_decision(
    request: &store::PermissionRequestData,
    question: &str,
    response: QuestionResponse<'_>,
) -> Result<PermissionRequestDecision, ApiError> {
    if request.tool_name.as_deref() != Some("AskUserQuestion") {
        return Err(ApiError::conflict("question request is no longer pending"));
    }
    let Some(questions) = request
        .tool_input
        .get("questions")
        .and_then(serde_json::Value::as_array)
    else {
        return Err(ApiError::conflict("question request is no longer pending"));
    };
    let Some(question_data) = questions
        .iter()
        .find(|item| item.get("question").and_then(serde_json::Value::as_str) == Some(question))
    else {
        return Err(ApiError::conflict("question request is no longer pending"));
    };
    let answer = match response {
        QuestionResponse::Choice(choice) => {
            if !(1..=9).contains(&choice) {
                return Err(ApiError::bad_request("choice must be between 1 and 9"));
            }
            let Some(label) = question_data
                .get("options")
                .and_then(serde_json::Value::as_array)
                .and_then(|options| options.get(usize::from(choice - 1)))
                .and_then(|option| option.get("label"))
                .and_then(serde_json::Value::as_str)
            else {
                return Err(ApiError::bad_request(
                    "choice is outside the available options",
                ));
            };
            label
        }
        QuestionResponse::Text(answer) => {
            let answer = answer.trim();
            if answer.is_empty() {
                return Err(ApiError::bad_request("answer must not be empty"));
            }
            answer
        }
    };
    Ok(PermissionRequestDecision::Allow {
        updated_input: Some(serde_json::json!({
            "questions": questions,
            "answers": { question: answer },
        })),
    })
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrimaryTaskRequest {
    task_id: String,
}

async fn set_primary_task(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
    Json(request): Json<PrimaryTaskRequest>,
) -> Result<Json<Success>, ApiError> {
    let session = load_session(state.clone(), id).await?;
    if request.task_id.trim().is_empty() {
        return Err(ApiError::bad_request("taskId cannot be empty"));
    }
    let sessions_dir = resolve_sessions_dir(state.sessions_dir).map_err(ApiError::internal)?;
    let changed = run_blocking(move || {
        store::set_watch_web_primary_task_in(&sessions_dir, &session.session_id, &request.task_id)
    })
    .await?;
    if !changed {
        return Err(ApiError::not_found());
    }
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
    session.status != SessionStatus::Ended
        && (session.has_active_work(session.status) || session.has_pending_permission_requests())
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
            if super::is_invalid_session_id_error(&error) {
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
    use crate::commands::agent::store::PermissionRequestWait;
    use crate::commands::agent::types::TmuxInfo;

    #[path = "send_message.rs"]
    mod send_message;

    #[path = "decisions.rs"]
    mod decisions;

    #[path = "proposals.rs"]
    mod proposals;

    struct FakeRuntime {
        calls: Mutex<Vec<Value>>,
        message_error: Mutex<Option<String>>,
        after_message: Mutex<Option<Box<dyn FnOnce() + Send>>>,
        message_gate: Mutex<
            Option<(
                tokio::sync::oneshot::Sender<()>,
                tokio::sync::oneshot::Receiver<()>,
            )>,
        >,
        pane_command: Mutex<Option<String>>,
        pane_has_agent_process: Mutex<bool>,
        pane_command_matches: Mutex<bool>,
    }

    impl Default for FakeRuntime {
        fn default() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                message_error: Mutex::new(None),
                after_message: Mutex::new(None),
                message_gate: Mutex::new(None),
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
            if let Some(after_message) = self.after_message.lock().unwrap().take() {
                after_message();
            }
            if let Some((started, release)) = self.message_gate.lock().unwrap().take() {
                let _ = started.send(());
                let _ = release.blocking_recv();
            }
            if let Some(message) = self.message_error.lock().unwrap().clone() {
                return Err(anyhow::anyhow!(message));
            }
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
            self.router_with_proposal_answer_locks().0
        }

        fn router_with_proposal_answer_locks(&self) -> (Router, ProposalAnswerLocks) {
            self.router_with_loaders_and_proposal_answer_locks(empty_message_loaders())
        }

        fn router_with_loaders_and_proposal_answer_locks(
            &self,
            message_loaders: super::super::messages::RouteLoaders,
        ) -> (Router, ProposalAnswerLocks) {
            let proposal_answer_locks = ProposalAnswerLocks::default();
            let router = router_with_state(ActionState {
                sessions_dir: Some(self.temp_dir.path().join("sessions")),
                attachment_dir: Some(self.temp_dir.path().join("attachments")),
                runtime: self.runtime.clone(),
                proposal_answer_locks: proposal_answer_locks.clone(),
                message_loaders,
            });
            (router, proposal_answer_locks)
        }

        fn save_session(&self, session: &Session) {
            store::save_session_to(&self.temp_dir.path().join("sessions"), session).unwrap();
        }

        fn save_pending_request(&self, session: &mut Session, tool_name: &str, tool_input: Value) {
            let request_id = "request-cobalt".to_string();
            session
                .pending_permission_agent_ids
                .insert(MAIN_THREAD_AGENT_KEY.to_string());
            session
                .pending_permission_request_ids
                .insert(MAIN_THREAD_AGENT_KEY.to_string(), request_id.clone());
            self.save_session(session);
            let lock = store::lock_session_for_update(
                &self.temp_dir.path().join("sessions"),
                &session.session_id,
            )
            .unwrap();
            lock.register_permission_request(
                MAIN_THREAD_AGENT_KEY,
                &request_id,
                Some(tool_name),
                &tool_input,
            )
            .unwrap();
            lock.save(session).unwrap();
        }

        fn take_permission_decision(&self) -> PermissionRequestDecision {
            let mut waiter =
                store::PermissionRequestWaiter::open(&self.temp_dir.path().join("sessions"))
                    .unwrap();
            let wait = waiter
                .poll("sample-session", MAIN_THREAD_AGENT_KEY, "request-cobalt")
                .unwrap();
            let PermissionRequestWait::Decision(decision) = wait else {
                panic!("expected a stored permission decision");
            };
            decision
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

        fn set_message_error(&self, message: &str) {
            *self.runtime.message_error.lock().unwrap() = Some(message.to_string());
        }

        fn set_after_message(&self, callback: impl FnOnce() + Send + 'static) {
            *self.runtime.after_message.lock().unwrap() = Some(Box::new(callback));
        }

        fn block_next_message(
            &self,
        ) -> (
            tokio::sync::oneshot::Receiver<()>,
            tokio::sync::oneshot::Sender<()>,
        ) {
            let (started, started_receiver) = tokio::sync::oneshot::channel();
            let (release, release_receiver) = tokio::sync::oneshot::channel();
            *self.runtime.message_gate.lock().unwrap() = Some((started, release_receiver));
            (started_receiver, release)
        }
    }

    #[fixture]
    fn action_context() -> ActionContext {
        ActionContext {
            temp_dir: TempDir::new().unwrap(),
            runtime: Arc::new(FakeRuntime::default()),
        }
    }

    fn empty_message_loaders() -> super::super::messages::RouteLoaders {
        super::super::messages::RouteLoaders::new(
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
            |_| Ok(Vec::new()),
        )
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
        let mut calls = context.runtime.calls.lock().unwrap().clone();
        for call in &mut calls {
            let Some(content) = call.get("content").and_then(Value::as_str) else {
                continue;
            };
            let Some((marker, _)) = content.split_once('\n') else {
                continue;
            };
            if marker.starts_with("⟦armyknife-user:")
                && marker.ends_with('⟧')
                && let Some(body) =
                    crate::commands::agent::watch_web_user_marker::strip_message(marker, content)
            {
                call["content"] = Value::String(body.to_owned());
            }
        }
        json!({
            "status": status,
            "body": serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null),
            "calls": calls,
        })
    }

    fn normalized_queued_prompts(
        context: &ActionContext,
        session_id: &str,
        attachment_path: Option<&str>,
    ) -> Value {
        let sessions_dir = context.temp_dir.path().join("sessions");
        let prompts = store::watch_web_queued_prompts_from(&sessions_dir, session_id).unwrap();
        Value::Array(
            prompts
                .into_iter()
                .map(|prompt| {
                    let content = attachment_path.map_or(prompt.content.clone(), |path| {
                        prompt.content.replace(path, "<attachment-path>")
                    });
                    json!({
                        "id": "<prompt-id>",
                        "content": content,
                        "sentAt": "<sent-at>",
                    })
                })
                .collect(),
        )
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
    #[case::first_choice(1, "Option A")]
    #[case::second_choice(2, "Option B")]
    #[case::third_choice(3, "Option C")]
    #[tokio::test]
    async fn answer_question_records_the_selected_answer(
        #[case] choice: u8,
        #[case] expected_answer: &str,
        action_context: ActionContext,
    ) {
        let mut session = session("sample-session");
        session.current_tool = Some("AskUserQuestion".to_string());
        action_context.save_pending_request(
            &mut session,
            "AskUserQuestion",
            json!({
                "questions": [{
                    "question": "Which output format?",
                    "options": [
                        { "label": "Option A" },
                        { "label": "Option B" },
                        { "label": "Option C" },
                    ],
                }],
            }),
        );

        let response = action_context
            .router()
            .oneshot(post(
                "/sessions/sample-session/answer",
                json!({ "question": "Which output format?", "choice": choice }),
            ))
            .await
            .unwrap();

        assert_eq!(
            json!({
                "response": response_snapshot(response, &action_context).await,
                "decision": serde_json::to_value(action_context.take_permission_decision()).unwrap(),
            }),
            json!({
                "response": {
                    "status": StatusCode::OK.as_u16(),
                    "body": { "ok": true },
                    "calls": [],
                },
                "decision": {
                    "behavior": "allow",
                    "updated_input": {
                        "questions": [{
                            "question": "Which output format?",
                            "options": [
                                { "label": "Option A" },
                                { "label": "Option B" },
                                { "label": "Option C" },
                            ],
                        }],
                        "answers": { "Which output format?": expected_answer },
                    },
                },
            }),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn answer_question_records_a_freeform_answer(action_context: ActionContext) {
        let mut session = session("sample-session");
        session.current_tool = Some("AskUserQuestion".to_string());
        action_context.save_pending_request(
            &mut session,
            "AskUserQuestion",
            json!({
                "questions": [{
                    "question": "Which output format?",
                    "options": [{ "label": "Option A" }],
                }],
            }),
        );

        let response = action_context
            .router()
            .oneshot(post(
                "/sessions/sample-session/answer",
                json!({
                    "question": "Which output format?",
                    "answer": "  A custom format  ",
                }),
            ))
            .await
            .unwrap();

        assert_eq!(
            json!({
                "response": response_snapshot(response, &action_context).await,
                "decision": serde_json::to_value(action_context.take_permission_decision()).unwrap(),
            }),
            json!({
                "response": {
                    "status": StatusCode::OK.as_u16(),
                    "body": { "ok": true },
                    "calls": [],
                },
                "decision": {
                    "behavior": "allow",
                    "updated_input": {
                        "questions": [{
                            "question": "Which output format?",
                            "options": [{ "label": "Option A" }],
                        }],
                        "answers": { "Which output format?": "A custom format" },
                    },
                },
            }),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn answer_question_skips_without_interrupting_the_turn(action_context: ActionContext) {
        let mut session = session("sample-session");
        session.current_tool = Some("AskUserQuestion".to_string());
        action_context.save_pending_request(
            &mut session,
            "AskUserQuestion",
            json!({ "questions": [{ "question": "Which output format?" }] }),
        );

        let response = action_context
            .router()
            .oneshot(post(
                "/sessions/sample-session/answer",
                json!({ "question": "Which output format?", "skip": true }),
            ))
            .await
            .unwrap();

        assert_eq!(
            json!({
                "response": response_snapshot(response, &action_context).await,
                "decision": serde_json::to_value(action_context.take_permission_decision()).unwrap(),
            }),
            json!({
                "response": {
                    "status": StatusCode::OK.as_u16(),
                    "body": { "ok": true },
                    "calls": [],
                },
                "decision": {
                    "behavior": "deny",
                    "message": "Question skipped from watch-web.",
                    "interrupt": false,
                },
            }),
        );
    }

    #[rstest]
    #[case::choice_with_skip(
        json!({ "question": "Which output format?", "skip": true, "choice": 1 }),
        "an answer cannot be provided when skipping a question",
    )]
    #[case::freeform_with_skip(
        json!({ "question": "Which output format?", "skip": true, "answer": "Option A" }),
        "an answer cannot be provided when skipping a question",
    )]
    #[case::choice_and_freeform(
        json!({ "question": "Which output format?", "choice": 1, "answer": "Option A" }),
        "choice and answer cannot both be provided",
    )]
    #[case::missing_choice(
        json!({ "question": "Which output format?" }),
        "choice or answer is required unless skipping a question",
    )]
    #[case::empty_freeform(
        json!({ "question": "Which output format?", "answer": "  " }),
        "answer must not be empty",
    )]
    #[tokio::test]
    async fn answer_question_rejects_invalid_requests(
        #[case] payload: Value,
        #[case] error: &str,
        action_context: ActionContext,
    ) {
        let mut session = session("sample-session");
        session.current_tool = Some("AskUserQuestion".to_string());
        action_context.save_pending_request(
            &mut session,
            "AskUserQuestion",
            json!({ "questions": [{ "question": "Which output format?" }] }),
        );

        let response = action_context
            .router()
            .oneshot(post("/sessions/sample-session/answer", payload))
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response, &action_context).await,
            json!({
                "status": StatusCode::BAD_REQUEST.as_u16(),
                "body": { "error": error },
                "calls": [],
            }),
        );
    }

    #[rstest]
    #[case::approve(true)]
    #[case::reject(false)]
    #[tokio::test]
    async fn permission_actions_record_a_hook_decision(
        #[case] approved: bool,
        action_context: ActionContext,
    ) {
        let mut session = session("sample-session");
        action_context.save_pending_request(
            &mut session,
            "Bash",
            json!({
                "command": "echo fictional",
            }),
        );
        let action = if approved { "approve" } else { "reject" };

        let response = action_context
            .router()
            .oneshot(post_empty(&format!(
                "/sessions/sample-session/permission/{action}"
            )))
            .await
            .unwrap();

        assert_eq!(
            json!({
                "response": response_snapshot(response, &action_context).await,
                "decision": serde_json::to_value(action_context.take_permission_decision()).unwrap(),
            }),
            json!({
                "response": {
                    "status": StatusCode::OK.as_u16(),
                    "body": { "ok": true },
                    "calls": [],
                },
                "decision": if approved {
                    json!({ "behavior": "allow" })
                } else {
                    json!({
                        "behavior": "deny",
                        "message": "Permission request rejected from watch-web.",
                        "interrupt": false,
                    })
                },
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
        action_context.save_pending_request(
            &mut session,
            "AskUserQuestion",
            json!({ "questions": [{ "question": "Which output format?", "options": [] }] }),
        );

        let response = action_context
            .router()
            .oneshot(post(
                "/sessions/sample-session/answer",
                json!({ "question": "Which output format?", "choice": choice }),
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
            .insert(MAIN_THREAD_AGENT_KEY.to_string());
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
    #[tokio::test]
    async fn primary_task_selection_is_persisted_and_publishes_a_revision(
        action_context: ActionContext,
    ) {
        let session = session("sample-session");
        action_context.save_session(&session);
        let sessions_dir = action_context.temp_dir.path().join("sessions");
        let before_revision = store::session_store_revision_from(&sessions_dir).unwrap();

        let response = action_context
            .router()
            .oneshot(post(
                "/sessions/sample-session/primary-task",
                json!({ "taskId": "sample-task" }),
            ))
            .await
            .unwrap();

        assert_eq!(
            (
                response_snapshot(response, &action_context).await,
                store::watch_web_session_states_from(&sessions_dir).unwrap(),
                store::session_store_revision_from(&sessions_dir).unwrap(),
            ),
            (
                json!({
                    "status": StatusCode::OK.as_u16(),
                    "body": { "ok": true },
                    "calls": [],
                }),
                vec![store::WatchWebSessionState {
                    session_id: "sample-session".to_string(),
                    later_until: None,
                    activity_snapshot: None,
                    until_activity: false,
                    is_today: false,
                    is_compacting: false,
                    primary_task_id: Some("sample-task".to_string()),
                }],
                before_revision + 1,
            ),
        );
    }

    #[rstest]
    #[case::empty("")]
    #[case::whitespace("  ")]
    #[tokio::test]
    async fn primary_task_selection_rejects_empty_task_ids(
        #[case] task_id: &str,
        action_context: ActionContext,
    ) {
        action_context.save_session(&session("sample-session"));

        let response = action_context
            .router()
            .oneshot(post(
                "/sessions/sample-session/primary-task",
                json!({ "taskId": task_id }),
            ))
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response, &action_context).await,
            json!({
                "status": StatusCode::BAD_REQUEST.as_u16(),
                "body": { "error": "taskId cannot be empty" },
                "calls": [],
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
    #[case::stopped(SessionStatus::Stopped, false)]
    #[case::ended_with_pending_task(SessionStatus::Ended, true)]
    #[tokio::test]
    async fn archive_hides_the_session_from_later_actions_lookups(
        #[case] status: SessionStatus,
        #[case] pending_task: bool,
        action_context: ActionContext,
    ) {
        let mut session = session("sample-session");
        session.status = status;
        if pending_task {
            session.pending_bg_task_ids.insert("task".to_string());
        }
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
    #[case::interrupt_missing("interrupt", None, true, true)]
    #[case::interrupt_shell("interrupt", Some("fish"), true, true)]
    #[case::interrupt_other_program("interrupt", Some("nvim"), false, true)]
    #[case::interrupt_command_changed("interrupt", Some("claude"), true, false)]
    #[tokio::test]
    async fn interrupt_rejects_unavailable_or_idle_panes(
        #[case] action: &str,
        #[case] pane_command: Option<&str>,
        #[case] pane_has_agent_process: bool,
        #[case] pane_command_matches: bool,
        action_context: ActionContext,
    ) {
        let session = session("sample-session");
        let (uri, body, keys) = match action {
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
            .insert(MAIN_THREAD_AGENT_KEY.to_string());
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
            .clone()
            .oneshot(post_empty("/sessions/sample-session/interrupt"))
            .await
            .unwrap();
        let message = app
            .oneshot(post(
                "/sessions/sample-session/messages",
                json!({ "content": "continue the investigation" }),
            ))
            .await
            .unwrap();

        assert_eq!(
            (
                response_snapshot(capabilities, &action_context).await,
                response_snapshot(answer, &action_context).await,
                response_snapshot(permission, &action_context).await,
                response_snapshot(interrupt, &action_context).await,
                response_snapshot(message, &action_context).await,
            ),
            (
                json!({
                    "status": StatusCode::OK.as_u16(),
                    "body": {
                        "message": false,
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
