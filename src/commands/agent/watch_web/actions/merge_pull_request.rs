use axum::Json;
use axum::extract::{Path as RoutePath, State};
use serde::Deserialize;

use super::{ActionState, ApiError, close::run_on_blocking_worker, load_session};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct MergePullRequestRequest {
    owner: String,
    repo: String,
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
    if !is_repository_component(&request.owner) || !is_repository_component(&request.repo) {
        return Err(ApiError::bad_request(
            "owner and repo must identify a canonical GitHub repository",
        ));
    }
    let session = load_session(state.clone(), id).await?;
    if session.status == super::SessionStatus::Ended {
        return Err(ApiError::conflict("session has ended"));
    }
    let runtime = state.runtime;
    run_on_blocking_worker(move || async move {
        runtime
            .merge_pull_request(
                &request.owner,
                &request.repo,
                number,
                &request.expected_head_sha,
            )
            .await
    })
    .await
    .map(Json)
    .map_err(ApiError::operation)
}

fn is_repository_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}
