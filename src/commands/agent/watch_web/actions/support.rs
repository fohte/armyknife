use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use serde::Serialize;

use super::super::super::types::{Session, TmuxInfo};
use super::super::store;
use super::{ActionRuntime, ActionState, ApiError};

#[derive(Serialize)]
pub(super) struct Success {
    ok: bool,
}

impl Success {
    pub(super) fn new() -> Self {
        Self { ok: true }
    }
}

pub(super) fn require_tmux(session: &Session) -> Result<&TmuxInfo, ApiError> {
    session
        .tmux_info
        .as_ref()
        .ok_or_else(|| ApiError::conflict("session has no tmux pane"))
}

pub(super) async fn load_session(state: ActionState, id: String) -> Result<Session, ApiError> {
    let session = tokio::task::spawn_blocking(move || {
        let session = match state.sessions_dir {
            Some(dir) => store::load_session_from(&dir, &id),
            None => crate::commands::agent::session_status::load_session_with_bg_run_status(&id),
        }
        .map_err(|error| {
            if super::super::is_invalid_session_id_error(&error) {
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

pub(super) fn resolve_sessions_dir(override_dir: Option<PathBuf>) -> Result<PathBuf> {
    match override_dir {
        Some(dir) => Ok(dir),
        None => store::sessions_dir(),
    }
}

pub(super) async fn resolve_current_session_id(
    state: &ActionState,
    session_id: &str,
) -> Result<String, ApiError> {
    let sessions_dir =
        resolve_sessions_dir(state.sessions_dir.clone()).map_err(ApiError::internal)?;
    let session_id = session_id.to_string();
    run_blocking(move || store::resolve_current_session_id_from(&sessions_dir, &session_id)).await
}

pub(super) async fn send_claude_escape(
    runtime: Arc<dyn ActionRuntime>,
    tmux_info: &TmuxInfo,
) -> Result<(), ApiError> {
    let tmux_info = tmux_info.clone();
    let result = tokio::task::spawn_blocking(move || {
        let Some(current_command) = runtime.pane_current_command(&tmux_info) else {
            return Ok(false);
        };
        if crate::infra::process::SHELL_COMMANDS.contains(&current_command.as_str()) {
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

pub(super) async fn run_blocking<T>(
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
