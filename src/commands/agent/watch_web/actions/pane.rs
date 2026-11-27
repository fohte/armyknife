use axum::Json;
use axum::extract::{Path as RoutePath, State};
use serde::Serialize;

use super::{ActionState, ApiError, load_session, require_tmux, run_blocking};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PaneSnapshotResponse {
    content: String,
    captured_at: String,
}

pub(super) async fn get_pane_snapshot(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
) -> Result<Json<PaneSnapshotResponse>, ApiError> {
    let session = load_session(state.clone(), id).await?;
    let tmux_info = require_tmux(&session)?.clone();
    let session_id = session.session_id.clone();
    let runtime = state.runtime;
    let content = run_blocking(move || runtime.capture_pane(&session_id, &tmux_info)).await?;
    let captured_at = chrono::Utc::now().to_rfc3339();

    Ok(Json(PaneSnapshotResponse {
        content,
        captured_at,
    }))
}
