use axum::Json;
use axum::extract::{Path as RoutePath, State};
use serde::Deserialize;

use super::super::super::session_title;
use super::{ActionState, ApiError, Success, load_session, resolve_sessions_dir, run_blocking};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SetTitleRequest {
    title: String,
}

pub(super) async fn set_title(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
    Json(request): Json<SetTitleRequest>,
) -> Result<Json<Success>, ApiError> {
    let session = load_session(state.clone(), id).await?;
    let sessions_dir = resolve_sessions_dir(state.sessions_dir).map_err(ApiError::internal)?;
    run_blocking(move || {
        session_title::set_session_title_in(&sessions_dir, &session.session_id, &request.title)
    })
    .await?;
    Ok(Json(Success::new()))
}
