use axum::Json;
use axum::extract::{Path as RoutePath, State};
use serde::Serialize;

use super::{ActionState, ApiError, load_session, require_tmux, run_blocking};

#[derive(Serialize)]
pub(super) struct PaneSnapshotResponse {
    content: String,
}

pub(super) async fn get_pane_snapshot(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
) -> Result<Json<PaneSnapshotResponse>, ApiError> {
    let session = load_session(state.clone(), id).await?;
    let tmux_info = require_tmux(&session)?.clone();
    let runtime = state.runtime;
    let content = run_blocking(move || runtime.capture_pane_with_escapes(&tmux_info)).await?;

    Ok(Json(PaneSnapshotResponse { content }))
}
