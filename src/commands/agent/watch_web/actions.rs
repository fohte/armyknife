use std::future::Future;
use std::path::Path;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;
use axum::extract::{DefaultBodyLimit, Path as RoutePath, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::super::resume::SHELL_COMMANDS;
use super::super::store;
use super::super::types::{Engine, Session, SessionStatus, TmuxInfo};
use super::messages::RouteLoaders;

mod answer_question;
mod api_error;
mod close;
mod decisions;
mod interrupt_locks;
mod keyed_mutex_registry;
mod merge_pull_request;
mod messages;
mod permission_denied;
mod permission_denied_locks;
mod proposal_answer_locks;
mod proposals;
mod rewind;
mod runtime;
mod title;
mod todos;

pub(super) use api_error::ApiError;
use interrupt_locks::InterruptLocks;
use permission_denied_locks::PermissionDeniedRetryLocks;
use proposal_answer_locks::ProposalAnswerLocks;
use runtime::LiveActionRuntime;

pub(super) fn router(message_loaders: RouteLoaders) -> Router {
    router_with_state(ActionState {
        sessions_dir: None,
        attachment_dir: None,
        runtime: Arc::new(LiveActionRuntime),
        proposal_answer_locks: ProposalAnswerLocks::default(),
        permission_denied_retry_locks: PermissionDeniedRetryLocks::default(),
        interrupt_locks: InterruptLocks::default(),
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
        .route("/sessions/{id}/rewind", post(rewind::rewind))
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
        .route(
            "/sessions/{id}/permission-denials/{seq}/retry",
            post(permission_denied::retry_permission_denied),
        )
        .route(
            "/sessions/{id}/todos/{seq}/resolve",
            post(todos::resolve_todo),
        )
        .route("/sessions/{id}/title", post(title::set_title))
        .route(
            "/sessions/{id}/answer",
            post(answer_question::answer_question),
        )
        .route(
            "/sessions/{id}/permission/approve",
            post(approve_permission),
        )
        .route("/sessions/{id}/permission/reject", post(reject_permission))
        .route("/sessions/{id}/interrupt", post(interrupt))
        .route("/sessions/{id}/focus", post(focus))
        .route("/sessions/{id}/read", post(mark_read))
        .route("/sessions/{id}/primary-task", post(set_primary_task))
        .route("/sessions/{id}/close", post(close::close))
        .route(
            "/sessions/{id}/pull-requests/{number}/merge",
            post(merge_pull_request::merge_pull_request),
        )
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
    permission_denied_retry_locks: PermissionDeniedRetryLocks,
    interrupt_locks: InterruptLocks,
}

trait ActionRuntime: Send + Sync {
    fn send_message(&self, session: &Session, content: &str) -> Result<()>;
    fn rewind_session(
        &self,
        sessions_dir: &Path,
        session: &Session,
        message_id: &str,
        mode: super::rewind::RewindMode,
    ) -> Result<String>;
    fn pane_current_command(&self, tmux_info: &TmuxInfo) -> Option<String>;
    fn send_claude_escape_if_agent_is_running(
        &self,
        tmux_info: &TmuxInfo,
        current_command: &str,
    ) -> Result<bool>;
    fn interrupt_codex(&self, thread_id: &str) -> Result<()>;
    fn focus(&self, tmux_info: &TmuxInfo) -> Result<()>;
    fn close_session<'a>(
        &'a self,
        session: &'a Session,
        keep_worktree: bool,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
    fn merge_pull_request<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        pr_number: u64,
        expected_head_sha: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<crate::commands::gh::WatchWebPrMergeResult>> + Send + 'a>>;
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
    rewind: bool,
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
            && session.has_pending_permission_requests(),
        permission: (has_live_session
            && session.engine == Engine::Claude
            && session.has_pending_permission_requests())
        .then_some(PermissionCapabilities {
            approve: true,
            reject: true,
        }),
        interrupt: session.status == SessionStatus::Running && has_tmux,
        open_in_terminal: has_tmux,
        rewind: super::rewind::can_rewind(&session),
    }))
}

async fn approve_permission(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
    request: Option<Json<PermissionRequestAction>>,
) -> Result<Json<Success>, ApiError> {
    let request_id = require_permission_request_id(request)?;
    resolve_permission_action(state, id, request_id, true).await
}

async fn reject_permission(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
    request: Option<Json<PermissionRequestAction>>,
) -> Result<Json<Success>, ApiError> {
    let request_id = require_permission_request_id(request)?;
    resolve_permission_action(state, id, request_id, false).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PermissionRequestAction {
    request_id: Option<String>,
}

fn require_permission_request_id(
    request: Option<Json<PermissionRequestAction>>,
) -> Result<String, ApiError> {
    request
        .and_then(|Json(request)| request.request_id)
        .filter(|request_id| !request_id.trim().is_empty())
        .ok_or_else(|| ApiError::bad_request("requestId is required"))
}

async fn resolve_permission_action(
    state: ActionState,
    id: String,
    request_id: String,
    approved: bool,
) -> Result<Json<Success>, ApiError> {
    let session = load_session(state.clone(), id).await?;
    if session.engine != Engine::Claude {
        return Err(ApiError::conflict(
            "permission approval is unavailable for this session",
        ));
    }
    if session.status == SessionStatus::Ended {
        return Err(ApiError::conflict("session has ended"));
    }
    let pending_request = session
        .pending_permission_request_ids
        .iter()
        .find(|(agent_key, pending_request_id)| {
            pending_request_id.as_str() == request_id
                && session
                    .pending_permission_agent_ids
                    .contains(agent_key.as_str())
        })
        .map(|(agent_key, pending_request_id)| (agent_key.clone(), pending_request_id.clone()));
    let Some((agent_key, request_id)) = pending_request else {
        return Err(ApiError::conflict(
            "permission request is no longer pending; answer it in the terminal",
        ));
    };
    let decision = if approved {
        store::PermissionRequestDecision::Allow
    } else {
        store::PermissionRequestDecision::Deny {
            message: "Rejected from watch-web.".to_string(),
            interrupt: false,
        }
    };
    let sessions_dir = resolve_sessions_dir(state.sessions_dir).map_err(ApiError::internal)?;
    let resolution = run_blocking(move || {
        store::resolve_permission_request(
            &sessions_dir,
            &session.session_id,
            &agent_key,
            &request_id,
            &decision,
        )
    })
    .await?;
    match resolution {
        store::PermissionRequestResolution::Resolved => {}
        store::PermissionRequestResolution::Missing
        | store::PermissionRequestResolution::Finished => {
            return Err(ApiError::conflict(
                "permission request is no longer pending; answer it in the terminal",
            ));
        }
    }
    Ok(Json(Success::new()))
}

async fn interrupt(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
) -> Result<Json<Success>, ApiError> {
    let session = load_session(state.clone(), id).await?;
    if session.status == SessionStatus::Ended {
        return Err(ApiError::conflict("session has ended"));
    }
    if session.status != SessionStatus::Running {
        return Err(ApiError::conflict("session is not running"));
    }
    let _guard = state
        .interrupt_locks
        .try_acquire(&session.session_id)
        .await
        .ok_or_else(|| ApiError::conflict("interrupt is already in progress"))?;

    let runtime = state.runtime;
    match session.engine {
        Engine::Codex => {
            let thread_id = session.session_id;
            run_blocking(move || runtime.interrupt_codex(&thread_id)).await?;
        }
        Engine::Claude => {
            let tmux_info = require_tmux(&session)?;
            send_claude_escape(runtime, tmux_info).await?;
        }
    }
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

async fn send_claude_escape(
    runtime: Arc<dyn ActionRuntime>,
    tmux_info: &TmuxInfo,
) -> Result<(), ApiError> {
    let tmux_info = tmux_info.clone();
    let result = tokio::task::spawn_blocking(move || {
        let Some(current_command) = runtime.pane_current_command(&tmux_info) else {
            return Ok(false);
        };
        if SHELL_COMMANDS.contains(&current_command.as_str()) {
            return Ok(false);
        }
        runtime.send_claude_escape_if_agent_is_running(&tmux_info, &current_command)
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
    use serde_json::{Value, json};
    use tempfile::TempDir;
    use tower::ServiceExt;

    use super::*;
    use crate::commands::agent::types::{MAIN_THREAD_AGENT_KEY, TmuxInfo};

    #[path = "send_message.rs"]
    mod send_message;

    #[path = "decisions.rs"]
    mod decisions;

    #[path = "proposals.rs"]
    mod proposals;

    #[path = "permission_denied.rs"]
    mod permission_denied;

    #[path = "todos.rs"]
    mod todos;

    #[path = "title.rs"]
    mod title;

    #[path = "merge_pull_request.rs"]
    mod merge_pull_request;

    struct FakeRuntime {
        calls: Mutex<Vec<Value>>,
        message_error: Mutex<Option<String>>,
        rewind_not_available: Mutex<bool>,
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
        close_unmerged_worktree: Mutex<bool>,
        sessions_dir: Mutex<Option<PathBuf>>,
        merge_error: Mutex<Option<String>>,
        merge_result: Mutex<crate::commands::gh::WatchWebPrMergeResult>,
        interrupt_gate: Mutex<
            Option<(
                tokio::sync::oneshot::Sender<()>,
                tokio::sync::oneshot::Receiver<()>,
            )>,
        >,
    }

    impl Default for FakeRuntime {
        fn default() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                message_error: Mutex::new(None),
                rewind_not_available: Mutex::new(false),
                after_message: Mutex::new(None),
                message_gate: Mutex::new(None),
                pane_command: Mutex::new(Some("claude".to_string())),
                pane_has_agent_process: Mutex::new(true),
                pane_command_matches: Mutex::new(true),
                close_unmerged_worktree: Mutex::new(false),
                sessions_dir: Mutex::new(None),
                merge_error: Mutex::new(None),
                merge_result: Mutex::new(crate::commands::gh::WatchWebPrMergeResult::Merged),
                interrupt_gate: Mutex::new(None),
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

        fn rewind_session(
            &self,
            _sessions_dir: &Path,
            session: &Session,
            message_id: &str,
            mode: super::super::rewind::RewindMode,
        ) -> Result<String> {
            self.calls.lock().unwrap().push(json!({
                "kind": "rewind",
                "sessionId": session.session_id,
                "messageId": message_id,
                "mode": format!("{mode:?}"),
            }));
            if *self.rewind_not_available.lock().unwrap() {
                return Err(super::super::rewind::SessionNotRewindableError.into());
            }
            Ok("fixture-fork-session".to_string())
        }

        fn pane_current_command(&self, tmux_info: &TmuxInfo) -> Option<String> {
            self.calls.lock().unwrap().push(json!({
                "kind": "pane-command",
                "paneId": tmux_info.pane_id,
                "socketPath": tmux_info.socket_path,
            }));
            self.pane_command.lock().unwrap().clone()
        }

        fn send_claude_escape_if_agent_is_running(
            &self,
            tmux_info: &TmuxInfo,
            current_command: &str,
        ) -> Result<bool> {
            let keys = ["Escape"];
            let pane_has_agent_process = *self.pane_has_agent_process.lock().unwrap();
            let pane_command_matches = *self.pane_command_matches.lock().unwrap();
            self.calls.lock().unwrap().push(json!({
                "kind": "guarded-keys",
                "paneId": tmux_info.pane_id,
                "engine": Engine::Claude,
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
            if let Some((started, release)) = self.interrupt_gate.lock().unwrap().take() {
                let _ = started.send(());
                let _ = release.blocking_recv();
            }
            Ok(true)
        }

        fn interrupt_codex(&self, thread_id: &str) -> Result<()> {
            self.calls.lock().unwrap().push(json!({
                "kind": "codex-interrupt",
                "threadId": thread_id,
            }));
            Ok(())
        }

        fn focus(&self, tmux_info: &TmuxInfo) -> Result<()> {
            self.calls.lock().unwrap().push(json!({
                "kind": "focus",
                "paneId": tmux_info.pane_id,
                "socketPath": tmux_info.socket_path,
            }));
            Ok(())
        }

        fn close_session<'a>(
            &'a self,
            session: &'a Session,
            keep_worktree: bool,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
            let fail_for_unmerged_worktree = *self.close_unmerged_worktree.lock().unwrap();
            let sessions_dir = self.sessions_dir.lock().unwrap().clone();
            let session_id = session.session_id.clone();
            self.calls.lock().unwrap().push(json!({
                "kind": "close",
                "sessionId": session.session_id,
                "keepWorktree": keep_worktree,
            }));
            Box::pin(async move {
                if fail_for_unmerged_worktree {
                    return Err(crate::shared::worktree_delete::UnmergedWorktreeError::new(
                        "sample-branch",
                        "not merged",
                    )
                    .into());
                }
                if let Some(sessions_dir) = sessions_dir {
                    store::update_session_in(&sessions_dir, &session_id, |stored| {
                        stored.status = SessionStatus::Ended;
                        true
                    })?;
                }
                Ok(())
            })
        }

        fn merge_pull_request<'a>(
            &'a self,
            owner: &'a str,
            repo: &'a str,
            pr_number: u64,
            expected_head_sha: &'a str,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<crate::commands::gh::WatchWebPrMergeResult>> + Send + 'a,
            >,
        > {
            self.calls.lock().unwrap().push(json!({
                "kind": "merge-pull-request",
                "owner": owner,
                "repo": repo,
                "number": pr_number,
                "expectedHeadSha": expected_head_sha,
            }));
            let error = self.merge_error.lock().unwrap().clone();
            let result = self.merge_result.lock().unwrap().clone();
            Box::pin(async move {
                if let Some(error) = error {
                    return Err(anyhow::anyhow!(error));
                }
                Ok(result)
            })
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

        fn router_with_permission_denied_retry_locks(
            &self,
        ) -> (Router, PermissionDeniedRetryLocks) {
            let permission_denied_retry_locks = PermissionDeniedRetryLocks::default();
            let router = router_with_state(ActionState {
                sessions_dir: Some(self.temp_dir.path().join("sessions")),
                attachment_dir: Some(self.temp_dir.path().join("attachments")),
                runtime: self.runtime.clone(),
                proposal_answer_locks: ProposalAnswerLocks::default(),
                permission_denied_retry_locks: permission_denied_retry_locks.clone(),
                interrupt_locks: InterruptLocks::default(),
                message_loaders: empty_message_loaders(),
            });
            (router, permission_denied_retry_locks)
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
                permission_denied_retry_locks: PermissionDeniedRetryLocks::default(),
                interrupt_locks: InterruptLocks::default(),
                message_loaders,
            });
            (router, proposal_answer_locks)
        }

        fn save_session(&self, session: &Session) {
            store::save_session_to(&self.temp_dir.path().join("sessions"), session).unwrap();
        }

        fn save_pending_permission_request(
            &self,
            session: &mut Session,
            agent_key: &str,
            request_id: &str,
            tool_name: &str,
        ) {
            self.save_pending_permission_request_with_input(
                session,
                agent_key,
                request_id,
                tool_name,
                &json!({}),
            );
        }

        fn save_pending_permission_request_with_input(
            &self,
            session: &mut Session,
            agent_key: &str,
            request_id: &str,
            tool_name: &str,
            tool_input: &Value,
        ) {
            session
                .pending_permission_agent_ids
                .insert(agent_key.to_string());
            session
                .pending_permission_request_ids
                .insert(agent_key.to_string(), request_id.to_string());
            let sessions_dir = self.temp_dir.path().join("sessions");
            let lock = store::lock_session_for_update(&sessions_dir, &session.session_id).unwrap();
            lock.register_permission_request(agent_key, request_id, Some(tool_name), tool_input)
                .unwrap();
            lock.save(session).unwrap();
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

        fn set_rewind_not_available(&self) {
            *self.runtime.rewind_not_available.lock().unwrap() = true;
        }

        fn set_close_unmerged_worktree(&self, value: bool) {
            *self.runtime.close_unmerged_worktree.lock().unwrap() = value;
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

        fn block_next_interrupt(
            &self,
        ) -> (
            tokio::sync::oneshot::Receiver<()>,
            tokio::sync::oneshot::Sender<()>,
        ) {
            let (started, started_receiver) = tokio::sync::oneshot::channel();
            let (release, release_receiver) = tokio::sync::oneshot::channel();
            *self.runtime.interrupt_gate.lock().unwrap() = Some((started, release_receiver));
            (started_receiver, release)
        }
    }

    #[fixture]
    fn action_context() -> ActionContext {
        let temp_dir = TempDir::new().unwrap();
        let runtime = Arc::new(FakeRuntime::default());
        *runtime.sessions_dir.lock().unwrap() = Some(temp_dir.path().join("sessions"));
        ActionContext { temp_dir, runtime }
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
            web_uis: Vec::new(),
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
            current_tool_name: None,
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
            let content =
                if let Some(marker_start) = content.rfind("<!-- armyknife-queued-prompt-id:") {
                    content[..marker_start]
                        .strip_suffix('\n')
                        .unwrap_or(&content[..marker_start])
                } else {
                    content
                };
            let Some((marker, _)) = content.split_once('\n') else {
                call["content"] = Value::String(content.to_owned());
                continue;
            };
            if marker.starts_with("⟦armyknife-user:")
                && marker.ends_with('⟧')
                && let Some(body) =
                    crate::commands::agent::watch_web_user_marker::strip_message(marker, content)
            {
                call["content"] = Value::String(body.to_owned());
            } else {
                call["content"] = Value::String(content.to_owned());
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

    fn question_request_input() -> Value {
        json!({
            "questions": [
                {
                    "question": "Which formats?",
                    "header": "Format",
                    "multiSelect": true,
                    "options": [{ "label": "Markdown" }, { "label": "Plain text" }],
                },
                {
                    "question": "Include examples?",
                    "header": "Examples",
                    "multiSelect": false,
                    "options": [{ "label": "Yes" }, { "label": "No" }],
                },
            ],
        })
    }

    fn question_answers() -> Value {
        json!({
            "Which formats?": "Markdown, Plain text",
            "Include examples?": "Yes",
        })
    }

    #[rstest]
    #[case::stopped(SessionStatus::Stopped)]
    #[case::paused(SessionStatus::Paused)]
    #[tokio::test]
    async fn rewind_dispatches_the_selected_user_message(
        #[case] status: SessionStatus,
        action_context: ActionContext,
    ) {
        let mut session = session("sample-session");
        session.status = status;
        action_context.save_session(&session);

        let response = action_context
            .router()
            .oneshot(post(
                "/sessions/sample-session/rewind",
                json!({
                    "messageId": "fixture-user-message",
                    "mode": "conversation"
                }),
            ))
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response, &action_context).await,
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "sessionId": "fixture-fork-session" },
                "calls": [{
                    "kind": "rewind",
                    "sessionId": "sample-session",
                    "messageId": "fixture-user-message",
                    "mode": "Conversation",
                }],
            }),
        );
    }

    #[rstest]
    #[case::running(SessionStatus::Running, false, false)]
    #[case::waiting_input(SessionStatus::WaitingInput, false, false)]
    #[case::ended(SessionStatus::Ended, false, false)]
    #[case::stopped_during_sweep(SessionStatus::Stopped, true, false)]
    #[case::stopped_with_pending_task(SessionStatus::Stopped, false, true)]
    #[tokio::test]
    async fn rewind_rejects_a_non_rewindable_session(
        #[case] status: SessionStatus,
        #[case] sweep_signaled: bool,
        #[case] has_pending_bg_task: bool,
        action_context: ActionContext,
    ) {
        let mut session = session("sample-session");
        session.status = status;
        session.sweep_signaled = sweep_signaled;
        if has_pending_bg_task {
            session
                .pending_bg_task_ids
                .insert("fixture-task".to_string());
        }
        action_context.save_session(&session);

        let response = action_context
            .router()
            .oneshot(post(
                "/sessions/sample-session/rewind",
                json!({
                    "messageId": "fixture-user-message",
                    "mode": "conversation"
                }),
            ))
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response, &action_context).await,
            json!({
                "status": StatusCode::CONFLICT.as_u16(),
                "body": { "error": "session cannot be rewound" },
                "calls": [],
            }),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn rewind_returns_conflict_when_session_becomes_unavailable(
        action_context: ActionContext,
    ) {
        let mut session = session("sample-session");
        session.status = SessionStatus::Stopped;
        action_context.save_session(&session);
        action_context.set_rewind_not_available();

        let response = action_context
            .router()
            .oneshot(post(
                "/sessions/sample-session/rewind",
                json!({
                    "messageId": "fixture-user-message",
                    "mode": "conversation"
                }),
            ))
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response, &action_context).await,
            json!({
                "status": StatusCode::CONFLICT.as_u16(),
                "body": { "error": "session cannot be rewound" },
                "calls": [{
                    "kind": "rewind",
                    "sessionId": "sample-session",
                    "messageId": "fixture-user-message",
                    "mode": "Conversation",
                }],
            }),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn codex_rewind_rejects_code_modes(action_context: ActionContext) {
        let mut session = session("sample-session");
        session.engine = Engine::Codex;
        session.status = SessionStatus::Stopped;
        action_context.save_session(&session);

        let response = action_context
            .router()
            .oneshot(post(
                "/sessions/sample-session/rewind",
                json!({
                    "messageId": "fixture-turn-id",
                    "mode": "code"
                }),
            ))
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response, &action_context).await,
            json!({
                "status": StatusCode::CONFLICT.as_u16(),
                "body": { "error": "Codex only supports conversation rewind" },
                "calls": [],
            }),
        );
    }

    #[rstest]
    #[case::main_thread("main", "question-request-main")]
    #[case::subagent("fixture-agent-id", "question-request-subagent")]
    #[tokio::test]
    async fn answer_question_writes_all_answers_for_the_matching_request(
        #[case] agent_key: &str,
        #[case] request_id: &str,
        action_context: ActionContext,
    ) {
        let mut session = session("sample-session");
        action_context.save_pending_permission_request_with_input(
            &mut session,
            agent_key,
            request_id,
            "AskUserQuestion",
            &question_request_input(),
        );

        let response = action_context
            .router()
            .oneshot(post(
                "/sessions/sample-session/answer",
                json!({ "requestId": request_id, "answers": question_answers() }),
            ))
            .await
            .unwrap();
        let mut waiter =
            store::PermissionRequestWaiter::open(&action_context.temp_dir.path().join("sessions"))
                .unwrap();

        assert_eq!(
            (
                response_snapshot(response, &action_context).await,
                waiter
                    .poll("sample-session", agent_key, request_id)
                    .unwrap(),
            ),
            (
                json!({
                    "status": StatusCode::OK.as_u16(),
                    "body": { "ok": true },
                    "calls": [],
                }),
                store::PermissionRequestWait::Decision {
                    decision: store::PermissionRequestDecision::Allow,
                    tool_input: question_request_input(),
                    answers: Some(question_answers()),
                },
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn answer_question_accepts_freeform_answers(action_context: ActionContext) {
        let mut session = session("sample-session");
        action_context.save_pending_permission_request_with_input(
            &mut session,
            MAIN_THREAD_AGENT_KEY,
            "question-request-main",
            "AskUserQuestion",
            &question_request_input(),
        );
        let answers = json!({
            "Which formats?": "Use a compact format, with a fictional label",
            "Include examples?": "Give one example per section",
        });

        let response = action_context
            .router()
            .oneshot(post(
                "/sessions/sample-session/answer",
                json!({ "requestId": "question-request-main", "answers": answers }),
            ))
            .await
            .unwrap();
        let mut waiter =
            store::PermissionRequestWaiter::open(&action_context.temp_dir.path().join("sessions"))
                .unwrap();

        assert_eq!(
            (
                response_snapshot(response, &action_context).await,
                waiter
                    .poll(
                        "sample-session",
                        MAIN_THREAD_AGENT_KEY,
                        "question-request-main"
                    )
                    .unwrap(),
            ),
            (
                json!({
                    "status": StatusCode::OK.as_u16(),
                    "body": { "ok": true },
                    "calls": [],
                }),
                store::PermissionRequestWait::Decision {
                    decision: store::PermissionRequestDecision::Allow,
                    tool_input: question_request_input(),
                    answers: Some(answers),
                },
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn answer_question_accepts_duplicate_question_texts(action_context: ActionContext) {
        let mut session = session("sample-session");
        let request_id = "question-request-duplicate-text";
        let tool_input = json!({
            "questions": [
                {
                    "question": "Which sections should be included?",
                    "options": [{ "label": "Overview" }, { "label": "Details" }],
                },
                {
                    "question": "Which sections should be included?",
                    "options": [{ "label": "Overview" }, { "label": "Details" }],
                },
            ],
        });
        let answers = json!({
            "Which sections should be included?": "Overview, Details",
        });
        action_context.save_pending_permission_request_with_input(
            &mut session,
            MAIN_THREAD_AGENT_KEY,
            request_id,
            "AskUserQuestion",
            &tool_input,
        );

        let response = action_context
            .router()
            .oneshot(post(
                "/sessions/sample-session/answer",
                json!({ "requestId": request_id, "answers": answers }),
            ))
            .await
            .unwrap();
        let mut waiter =
            store::PermissionRequestWaiter::open(&action_context.temp_dir.path().join("sessions"))
                .unwrap();

        assert_eq!(
            (
                response_snapshot(response, &action_context).await,
                waiter
                    .poll("sample-session", MAIN_THREAD_AGENT_KEY, request_id)
                    .unwrap(),
            ),
            (
                json!({
                    "status": StatusCode::OK.as_u16(),
                    "body": { "ok": true },
                    "calls": [],
                }),
                store::PermissionRequestWait::Decision {
                    decision: store::PermissionRequestDecision::Allow,
                    tool_input,
                    answers: Some(answers),
                },
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn skipping_a_question_denies_without_interrupting_the_turn(
        action_context: ActionContext,
    ) {
        let mut session = session("sample-session");
        action_context.save_pending_permission_request_with_input(
            &mut session,
            MAIN_THREAD_AGENT_KEY,
            "question-request-main",
            "AskUserQuestion",
            &question_request_input(),
        );

        let response = action_context
            .router()
            .oneshot(post(
                "/sessions/sample-session/answer",
                json!({ "requestId": "question-request-main", "skip": true }),
            ))
            .await
            .unwrap();
        let mut waiter =
            store::PermissionRequestWaiter::open(&action_context.temp_dir.path().join("sessions"))
                .unwrap();

        assert_eq!(
            (
                response_snapshot(response, &action_context).await,
                waiter
                    .poll(
                        "sample-session",
                        MAIN_THREAD_AGENT_KEY,
                        "question-request-main"
                    )
                    .unwrap(),
            ),
            (
                json!({
                    "status": StatusCode::OK.as_u16(),
                    "body": { "ok": true },
                    "calls": [],
                }),
                store::PermissionRequestWait::Decision {
                    decision: store::PermissionRequestDecision::Deny {
                        message: "The user skipped this question in watch-web.".to_string(),
                        interrupt: false,
                    },
                    tool_input: question_request_input(),
                    answers: None,
                },
            ),
        );
    }

    #[rstest]
    #[case::request_id_missing(json!({}), "requestId is required")]
    #[case::answers_missing(
        json!({ "requestId": "question-request-main" }),
        "answers are required unless skipping",
    )]
    #[case::skip_with_answers(
        json!({ "requestId": "question-request-main", "skip": true, "answers": {} }),
        "answers cannot be provided when skipping a question",
    )]
    #[case::incomplete_answers(
        json!({ "requestId": "question-request-main", "answers": { "Which formats?": "Markdown" } }),
        "an answer is required for every question",
    )]
    #[tokio::test]
    async fn answer_question_rejects_incomplete_requests(
        #[case] payload: Value,
        #[case] error: &str,
        action_context: ActionContext,
    ) {
        let mut session = session("sample-session");
        action_context.save_pending_permission_request_with_input(
            &mut session,
            MAIN_THREAD_AGENT_KEY,
            "question-request-main",
            "AskUserQuestion",
            &question_request_input(),
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
    #[tokio::test]
    async fn terminal_resolved_question_request_returns_a_terminal_fallback_conflict(
        action_context: ActionContext,
    ) {
        let mut session = session("sample-session");
        action_context.save_pending_permission_request_with_input(
            &mut session,
            MAIN_THREAD_AGENT_KEY,
            "question-request-main",
            "AskUserQuestion",
            &question_request_input(),
        );
        session.pending_permission_agent_ids.clear();
        session.pending_permission_request_ids.clear();
        action_context.save_session(&session);

        let response = action_context
            .router()
            .oneshot(post(
                "/sessions/sample-session/answer",
                json!({ "requestId": "question-request-main", "answers": question_answers() }),
            ))
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response, &action_context).await,
            json!({
                "status": StatusCode::CONFLICT.as_u16(),
                "body": { "error": "question request is no longer pending; answer it in the terminal" },
                "calls": [],
            }),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn expired_question_request_returns_a_terminal_fallback_conflict(
        action_context: ActionContext,
    ) {
        let mut session = session("sample-session");
        session
            .pending_permission_agent_ids
            .insert(MAIN_THREAD_AGENT_KEY.to_string());
        session.pending_permission_request_ids.insert(
            MAIN_THREAD_AGENT_KEY.to_string(),
            "question-request-main".to_string(),
        );
        action_context.save_session(&session);

        let response = action_context
            .router()
            .oneshot(post(
                "/sessions/sample-session/answer",
                json!({ "requestId": "question-request-main", "answers": question_answers() }),
            ))
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response, &action_context).await,
            json!({
                "status": StatusCode::CONFLICT.as_u16(),
                "body": { "error": "question request is no longer pending; answer it in the terminal" },
                "calls": [],
            }),
        );
    }

    #[rstest]
    #[case::approve(true, store::PermissionRequestDecision::Allow)]
    #[case::reject(
        false,
        store::PermissionRequestDecision::Deny {
            message: "Rejected from watch-web.".to_string(),
            interrupt: false,
        }
    )]
    #[tokio::test]
    async fn permission_actions_write_the_hook_decision(
        #[case] approved: bool,
        #[case] expected_decision: store::PermissionRequestDecision,
        action_context: ActionContext,
    ) {
        let mut session = session("sample-session");
        action_context.save_pending_permission_request(
            &mut session,
            "main",
            "permission-request-main",
            "Bash",
        );
        let action = if approved { "approve" } else { "reject" };

        let response = action_context
            .router()
            .oneshot(post(
                &format!("/sessions/sample-session/permission/{action}"),
                json!({ "requestId": "permission-request-main" }),
            ))
            .await
            .unwrap();
        let mut waiter =
            store::PermissionRequestWaiter::open(&action_context.temp_dir.path().join("sessions"))
                .unwrap();

        assert_eq!(
            (
                response_snapshot(response, &action_context).await,
                waiter
                    .poll("sample-session", "main", "permission-request-main")
                    .unwrap(),
            ),
            (
                json!({
                    "status": StatusCode::OK.as_u16(),
                    "body": { "ok": true },
                    "calls": [],
                }),
                store::PermissionRequestWait::Decision {
                    decision: expected_decision,
                    tool_input: json!({}),
                    answers: None,
                },
            ),
        );
    }

    #[rstest]
    #[case::approve_without_body("approve", None)]
    #[case::approve_without_request_id("approve", Some(json!({})))]
    #[case::approve_with_empty_request_id(
        "approve",
        Some(json!({ "requestId": "" })),
    )]
    #[case::approve_with_whitespace_request_id(
        "approve",
        Some(json!({ "requestId": "   " })),
    )]
    #[case::reject_without_body("reject", None)]
    #[case::reject_without_request_id("reject", Some(json!({})))]
    #[case::reject_with_empty_request_id("reject", Some(json!({ "requestId": "" })))]
    #[case::reject_with_whitespace_request_id("reject", Some(json!({ "requestId": "   " })))]
    #[tokio::test]
    async fn permission_actions_require_a_request_id(
        #[case] action: &str,
        #[case] payload: Option<Value>,
        action_context: ActionContext,
    ) {
        let mut session = session("sample-session");
        action_context.save_pending_permission_request(
            &mut session,
            MAIN_THREAD_AGENT_KEY,
            "permission-request-main",
            "Bash",
        );

        let uri = format!("/sessions/sample-session/permission/{action}");
        let request = payload.map_or_else(|| post_empty(&uri), |payload| post(&uri, payload));
        let response = action_context.router().oneshot(request).await.unwrap();
        let mut waiter =
            store::PermissionRequestWaiter::open(&action_context.temp_dir.path().join("sessions"))
                .unwrap();

        assert_eq!(
            (
                response_snapshot(response, &action_context).await,
                waiter
                    .poll(
                        "sample-session",
                        MAIN_THREAD_AGENT_KEY,
                        "permission-request-main",
                    )
                    .unwrap(),
            ),
            (
                json!({
                    "status": StatusCode::BAD_REQUEST.as_u16(),
                    "body": { "error": "requestId is required" },
                    "calls": [],
                }),
                store::PermissionRequestWait::Pending,
            ),
        );
    }

    #[tokio::test]
    async fn permission_capability_is_enabled_for_a_subagent_only_request() {
        let action_context = action_context();
        let mut session = session("sample-session");
        session.tmux_info = None;
        action_context.save_pending_permission_request_with_input(
            &mut session,
            "agent-fixture",
            "permission-request-subagent",
            "AskUserQuestion",
            &question_request_input(),
        );

        let response = action_context
            .router()
            .oneshot(
                Request::builder()
                    .uri("/sessions/sample-session/actions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response, &action_context).await,
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": {
                    "message": true,
                    "answerQuestion": true,
                    "permission": { "approve": true, "reject": true },
                    "interrupt": false,
                    "openInTerminal": false,
                    "rewind": false,
                },
                "calls": [],
            }),
        );
    }

    #[tokio::test]
    async fn permission_action_resolves_only_the_requested_subagent() {
        let action_context = action_context();
        let mut session = session("sample-session");
        action_context.save_pending_permission_request(
            &mut session,
            "main",
            "permission-request-main",
            "Bash",
        );
        action_context.save_pending_permission_request(
            &mut session,
            "agent-fixture-a",
            "permission-request-a",
            "Write",
        );
        action_context.save_pending_permission_request(
            &mut session,
            "agent-fixture-b",
            "permission-request-b",
            "Bash",
        );

        let response = action_context
            .router()
            .oneshot(post(
                "/sessions/sample-session/permission/reject",
                json!({ "requestId": "permission-request-b" }),
            ))
            .await
            .unwrap();
        let sessions_dir = action_context.temp_dir.path().join("sessions");
        let mut waiter = store::PermissionRequestWaiter::open(&sessions_dir).unwrap();

        assert_eq!(
            (
                response_snapshot(response, &action_context).await,
                waiter
                    .poll("sample-session", "agent-fixture-b", "permission-request-b")
                    .unwrap(),
                store::permission_request_data(
                    &sessions_dir,
                    "sample-session",
                    "permission-request-main",
                )
                .unwrap(),
                store::permission_request_data(
                    &sessions_dir,
                    "sample-session",
                    "permission-request-a",
                )
                .unwrap(),
            ),
            (
                json!({
                    "status": StatusCode::OK.as_u16(),
                    "body": { "ok": true },
                    "calls": [],
                }),
                store::PermissionRequestWait::Decision {
                    decision: store::PermissionRequestDecision::Deny {
                        message: "Rejected from watch-web.".to_string(),
                        interrupt: false,
                    },
                    tool_input: json!({}),
                    answers: None,
                },
                Some(store::PermissionRequestData {
                    tool_name: Some("Bash".to_string()),
                    tool_input: json!({}),
                    pending: true,
                }),
                Some(store::PermissionRequestData {
                    tool_name: Some("Write".to_string()),
                    tool_input: json!({}),
                    pending: true,
                }),
            ),
        );
    }

    #[tokio::test]
    async fn permission_action_after_hook_auto_deny_returns_conflict() {
        let action_context = action_context();
        let mut session = session("sample-session");
        action_context.save_pending_permission_request(
            &mut session,
            "main",
            "permission-request-main",
            "Bash",
        );
        let mut waiter =
            store::PermissionRequestWaiter::open(&action_context.temp_dir.path().join("sessions"))
                .unwrap();
        waiter
            .expire_with_deny(
                "sample-session",
                "main",
                "permission-request-main",
                "automatic denial fixture",
            )
            .unwrap();

        let response = action_context
            .router()
            .oneshot(post(
                "/sessions/sample-session/permission/approve",
                json!({ "requestId": "permission-request-main" }),
            ))
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response, &action_context).await,
            json!({
                "status": StatusCode::CONFLICT.as_u16(),
                "body": {
                    "error": "permission request is no longer pending; answer it in the terminal"
                },
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
        session.current_tool_name = Some("AskUserQuestion".to_string());
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
                json!({ "requestId": "missing-request" }),
            ))
            .await
            .unwrap();
        let answer = app
            .oneshot(post(
                "/sessions/sample-session/answer",
                json!({ "requestId": "question-request-main", "skip": true }),
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
                        "interrupt": false,
                        "openInTerminal": true,
                        "rewind": false,
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
    #[case::running(SessionStatus::Running, true)]
    #[case::waiting_input(SessionStatus::WaitingInput, false)]
    #[case::stopped(SessionStatus::Stopped, false)]
    #[tokio::test]
    async fn interrupt_capability_is_available_only_while_running(
        #[case] status: SessionStatus,
        #[case] enabled: bool,
        action_context: ActionContext,
    ) {
        let mut session = session("sample-session");
        session.status = status;
        action_context.save_session(&session);

        let response = action_context
            .router()
            .oneshot(
                Request::builder()
                    .uri("/sessions/sample-session/actions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response, &action_context).await,
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": {
                    "message": true,
                    "answerQuestion": false,
                    "interrupt": enabled,
                    "openInTerminal": true,
                    "rewind": matches!(status, SessionStatus::Stopped | SessionStatus::Paused),
                },
                "calls": [],
            }),
        );
    }

    #[rstest]
    #[case::running_with_pane(SessionStatus::Running, true, false, false, false)]
    #[case::waiting_input_with_pane(SessionStatus::WaitingInput, true, false, false, false)]
    #[case::stopped_with_pane(SessionStatus::Stopped, true, true, false, false)]
    #[case::paused_with_pane(SessionStatus::Paused, true, true, false, false)]
    #[case::ended_with_pane(SessionStatus::Ended, true, false, false, false)]
    #[case::stopped_without_pane(SessionStatus::Stopped, false, false, false, false)]
    #[case::paused_without_pane(SessionStatus::Paused, false, false, false, false)]
    #[case::waiting_input_without_pane(SessionStatus::WaitingInput, false, false, false, false)]
    #[case::stopped_with_pending_task(SessionStatus::Stopped, true, false, true, false)]
    #[case::stopped_during_sweep(SessionStatus::Stopped, true, false, false, true)]
    #[tokio::test]
    async fn rewind_capability_requires_a_stopped_or_paused_session_with_a_pane(
        #[case] status: SessionStatus,
        #[case] has_pane: bool,
        #[case] rewind_available: bool,
        #[case] has_pending_bg_task: bool,
        #[case] sweep_signaled: bool,
        action_context: ActionContext,
    ) {
        let mut session = session("sample-session");
        session.status = status;
        if has_pending_bg_task {
            session
                .pending_bg_task_ids
                .insert("fixture-task".to_string());
        }
        session.sweep_signaled = sweep_signaled;
        if !has_pane {
            session.tmux_info = None;
        }
        action_context.save_session(&session);

        let response = action_context
            .router()
            .oneshot(
                Request::builder()
                    .uri("/sessions/sample-session/actions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response, &action_context).await,
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": {
                    "message": status != SessionStatus::Ended,
                    "answerQuestion": false,
                    "interrupt": status == SessionStatus::Running,
                    "openInTerminal": has_pane,
                    "rewind": rewind_available,
                },
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
    async fn interrupt_uses_engine_specific_strategy(
        #[case] engine: Engine,
        #[case] current_command: &str,
        action_context: ActionContext,
    ) {
        let mut session = session("sample-session");
        session.engine = engine;
        session.status = SessionStatus::Running;
        action_context.save_session(&session);
        let expected_calls = match engine {
            Engine::Claude => {
                action_context.set_pane_command(Some(current_command));
                json!([{
                    "kind": "pane-command",
                    "paneId": "%4",
                    "socketPath": "/tmp/example-tmux.sock",
                }, {
                    "kind": "guarded-keys",
                    "paneId": "%4",
                    "engine": "claude",
                    "currentCommand": current_command,
                    "keys": ["Escape"],
                    "paneHasAgentProcess": true,
                    "paneCommandMatches": true,
                }, {
                    "kind": "keys",
                    "paneId": "%4",
                    "socketPath": "/tmp/example-tmux.sock",
                    "keys": ["Escape"],
                }])
            }
            Engine::Codex => json!([{
                "kind": "codex-interrupt",
                "threadId": "sample-session",
            }]),
        };

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
                "calls": expected_calls,
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
    async fn close_ends_the_session_without_archiving_it(action_context: ActionContext) {
        let mut session = session("sample-session");
        session.status = SessionStatus::Stopped;
        action_context.save_session(&session);

        let closed = action_context
            .router()
            .oneshot(post_empty("/sessions/sample-session/close"))
            .await
            .unwrap();
        session.status = SessionStatus::Ended;
        assert_eq!(
            (
                response_snapshot(closed, &action_context).await,
                normalize_session(action_context.load_session("sample-session")),
            ),
            (
                json!({ "status": StatusCode::OK.as_u16(), "body": { "ok": true }, "calls": [{
                    "kind": "close",
                    "sessionId": "sample-session",
                    "keepWorktree": false,
                }] }),
                normalize_session(session),
            ),
        );
    }

    #[rstest]
    #[case::missing_pane_command(None, true, true)]
    #[case::shell(Some("fish"), true, true)]
    #[case::other_program(Some("nvim"), false, true)]
    #[case::command_changed(Some("claude"), true, false)]
    #[tokio::test]
    async fn running_claude_interrupt_rejects_unavailable_or_idle_panes(
        #[case] pane_command: Option<&str>,
        #[case] pane_has_agent_process: bool,
        #[case] pane_command_matches: bool,
        action_context: ActionContext,
    ) {
        let mut session = session("sample-session");
        session.status = SessionStatus::Running;
        action_context.save_session(&session);
        action_context.set_pane_command(pane_command);
        action_context.set_pane_has_agent_process(pane_has_agent_process);
        action_context.set_pane_command_matches(pane_command_matches);

        let response = action_context
            .router()
            .oneshot(post_empty("/sessions/sample-session/interrupt"))
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
                "keys": ["Escape"],
                "paneHasAgentProcess": pane_has_agent_process,
                "paneCommandMatches": pane_command_matches,
            }));
            if pane_has_agent_process && pane_command_matches {
                expected_calls.push(json!({
                    "kind": "keys",
                    "paneId": "%4",
                    "socketPath": "/tmp/example-tmux.sock",
                    "keys": ["Escape"],
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
    #[case::claude_waiting_input(Engine::Claude, SessionStatus::WaitingInput)]
    #[case::codex_waiting_input(Engine::Codex, SessionStatus::WaitingInput)]
    #[case::claude_stopped(Engine::Claude, SessionStatus::Stopped)]
    #[tokio::test]
    async fn interrupt_rejects_sessions_that_are_not_running(
        #[case] engine: Engine,
        #[case] status: SessionStatus,
        action_context: ActionContext,
    ) {
        let mut session = session("sample-session");
        session.engine = engine;
        session.status = status;
        action_context.save_session(&session);

        let response = action_context
            .router()
            .oneshot(post_empty("/sessions/sample-session/interrupt"))
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response, &action_context).await,
            json!({
                "status": StatusCode::CONFLICT.as_u16(),
                "body": { "error": "session is not running" },
                "calls": [],
            }),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn concurrent_interrupt_is_rejected_while_the_first_is_in_flight(
        action_context: ActionContext,
    ) {
        let mut session = session("sample-session");
        session.engine = Engine::Claude;
        session.status = SessionStatus::Running;
        action_context.save_session(&session);
        let (started, release) = action_context.block_next_interrupt();
        let app = action_context.router();
        let first_app = app.clone();
        let first = tokio::spawn(async move {
            first_app
                .oneshot(post_empty("/sessions/sample-session/interrupt"))
                .await
                .unwrap()
        });
        started.await.unwrap();

        let second = app
            .oneshot(post_empty("/sessions/sample-session/interrupt"))
            .await
            .unwrap();
        let second_snapshot = response_snapshot(second, &action_context).await;
        let _ = release.send(());
        let first_snapshot = response_snapshot(first.await.unwrap(), &action_context).await;

        let expected = json!({
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
                "keys": ["Escape"],
                "paneHasAgentProcess": true,
                "paneCommandMatches": true,
            }, {
                "kind": "keys",
                "paneId": "%4",
                "socketPath": "/tmp/example-tmux.sock",
                "keys": ["Escape"],
            }],
        });
        assert_eq!(
            (first_snapshot, second_snapshot),
            (
                expected.clone(),
                json!({
                    "status": StatusCode::CONFLICT.as_u16(),
                    "body": { "error": "interrupt is already in progress" },
                    "calls": expected["calls"],
                }),
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn ended_sessions_hide_and_reject_key_actions(action_context: ActionContext) {
        let mut session = session("sample-session");
        session.status = SessionStatus::Ended;
        session.current_tool = Some("AskUserQuestion".to_string());
        session.current_tool_name = Some("AskUserQuestion".to_string());
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
                json!({ "requestId": "question-request-main", "skip": true }),
            ))
            .await
            .unwrap();
        let permission = app
            .clone()
            .oneshot(post(
                "/sessions/sample-session/permission/approve",
                json!({ "requestId": "permission-request-main" }),
            ))
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
                        "rewind": false,
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
    #[tokio::test]
    async fn close_forwards_keep_worktree_choice_to_common_close(action_context: ActionContext) {
        let mut session = session("sample-session");
        session.status = SessionStatus::Stopped;
        action_context.save_session(&session);
        let response = action_context
            .router()
            .oneshot(post(
                "/sessions/sample-session/close",
                json!({ "keepWorktree": true }),
            ))
            .await
            .unwrap();

        let response = response_snapshot(response, &action_context).await;
        session.status = SessionStatus::Ended;
        assert_eq!(
            json!({
                "response": response,
                "session": normalize_session(action_context.load_session("sample-session")),
            }),
            json!({
                "response": {
                    "status": StatusCode::OK.as_u16(),
                    "body": { "ok": true },
                    "calls": [{
                        "kind": "close",
                        "sessionId": "sample-session",
                        "keepWorktree": true,
                    }],
                },
                "session": normalize_session(session),
            }),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn close_returns_unmerged_worktree_choice_with_the_reason(action_context: ActionContext) {
        action_context.save_session(&session("sample-session"));
        action_context.set_close_unmerged_worktree(true);

        let response = action_context
            .router()
            .oneshot(post_empty("/sessions/sample-session/close"))
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response, &action_context).await,
            json!({
                "status": StatusCode::CONFLICT.as_u16(),
                "body": {
                    "error": "Branch `sample-branch` is not merged (not merged); close the session while keeping its worktree to continue",
                    "code": "unmerged_worktree",
                },
                "calls": [{
                    "kind": "close",
                    "sessionId": "sample-session",
                    "keepWorktree": false,
                }],
            }),
        );
    }
}
