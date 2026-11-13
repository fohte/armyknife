use anyhow::Result;
use axum::Json;
use axum::extract::{Path as RoutePath, State};
use serde::{Deserialize, Serialize};

use super::super::rewind::RewindMode;
use super::{ActionState, ApiError, load_session, resolve_sessions_dir, run_blocking};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RewindRequest {
    message_id: String,
    mode: RewindMode,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RewindResponse {
    session_id: String,
}

pub(super) async fn rewind(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
    Json(request): Json<RewindRequest>,
) -> Result<Json<RewindResponse>, ApiError> {
    let _session_action_guard = super::messages::lock_session_action(&id).await;
    let session = load_session(state.clone(), id).await?;
    if !super::super::rewind::can_rewind(&session) {
        return Err(ApiError::conflict("session is not waiting for input"));
    }
    if session.engine == crate::commands::agent::types::Engine::Codex
        && request.mode != RewindMode::Conversation
    {
        return Err(ApiError::conflict(
            "Codex only supports conversation rewind",
        ));
    }
    if request.message_id.trim().is_empty() {
        return Err(ApiError::bad_request("messageId is required"));
    }

    let sessions_dir =
        resolve_sessions_dir(state.sessions_dir.clone()).map_err(ApiError::internal)?;
    let runtime = state.runtime;
    let message_id = request.message_id;
    let mode = request.mode;
    let session_id =
        run_blocking(move || runtime.rewind_session(&sessions_dir, &session, &message_id, mode))
            .await?;
    Ok(Json(RewindResponse { session_id }))
}
