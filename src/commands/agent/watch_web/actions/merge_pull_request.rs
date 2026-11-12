use axum::Json;
use axum::extract::{Path as RoutePath, State};
use serde::Deserialize;

use super::{ActionState, ApiError, close::run_on_blocking_worker, load_session};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct MergePullRequestRequest {
    expected_head_sha: String,
}

pub(super) async fn merge_pull_request(
    State(state): State<ActionState>,
    RoutePath((id, number)): RoutePath<(String, u64)>,
    Json(request): Json<MergePullRequestRequest>,
) -> Result<Json<crate::commands::gh::WatchWebPrMergeResult>, ApiError> {
    if request.expected_head_sha.trim().is_empty() {
        return Err(ApiError::bad_request("expectedHeadSha cannot be empty"));
    }
    let session = load_session(state.clone(), id).await?;
    if session.status == super::SessionStatus::Ended {
        return Err(ApiError::conflict("session has ended"));
    }
    let runtime = state.runtime;
    run_on_blocking_worker(move || async move {
        runtime
            .merge_pull_request(&session, number, &request.expected_head_sha)
            .await
    })
    .await
    .map(Json)
    .map_err(ApiError::operation)
}
