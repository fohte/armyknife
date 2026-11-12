use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;

use super::super::session_enrichment::SessionTaskMetadata;
use super::task_selection::primary_task_id;
use super::{PullRequestListResponse, SessionSource};

pub(super) async fn session_pull_requests<S: SessionSource>(
    State(source): State<S>,
    Path(session_id): Path<String>,
) -> Result<Json<PullRequestListResponse>, StatusCode> {
    let snapshot = source.load_snapshot().await.map_err(|error| {
        tracing::warn!(%error, "failed to load sessions for PR list");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let task_metadata = source.fetch_task_metadata(&snapshot.all_sessions).await;
    let task_ids_by_session = task_ids_by_session(&task_metadata);
    let linked_tasks = task_metadata.links.get(&session_id);
    let saved_task_id = snapshot
        .web_states
        .iter()
        .find(|state| state.session_id == session_id)
        .and_then(|state| state.primary_task_id.as_deref());
    let task_id = primary_task_id(
        saved_task_id,
        linked_tasks
            .into_iter()
            .flatten()
            .map(|task| task.task_id.as_str()),
    )
    .map(str::to_owned);

    let response = source
        .fetch_session_pull_request_list(
            session_id,
            task_id,
            snapshot.all_sessions,
            task_ids_by_session,
            snapshot.agent_config,
        )
        .await
        .map_err(|error| {
            tracing::warn!(%error, "failed to fetch session PR list");
            StatusCode::BAD_GATEWAY
        })?;

    Ok(Json(response))
}

fn task_ids_by_session(task_metadata: &SessionTaskMetadata) -> HashMap<String, Vec<String>> {
    task_metadata
        .links
        .iter()
        .map(|(session_id, tasks)| {
            (
                session_id.clone(),
                tasks.iter().map(|task| task.task_id.clone()).collect(),
            )
        })
        .collect()
}
