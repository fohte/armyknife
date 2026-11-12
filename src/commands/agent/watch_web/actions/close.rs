use std::future::Future;

use axum::Json;
use axum::extract::{Path as RoutePath, State};
use serde::Deserialize;

use super::*;

pub(super) async fn run_close_on_blocking_worker<F, Fut>(operation: F) -> Result<()>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    let runtime = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || runtime.block_on(operation()))
        .await
        .map_err(anyhow::Error::from)?
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(super) struct CloseRequest {
    keep_worktree: bool,
}

pub(super) async fn close(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
    request: Option<Json<CloseRequest>>,
) -> Result<Json<Success>, ApiError> {
    let session = load_session(state.clone(), id).await?;
    state
        .runtime
        .close_session(
            &session,
            request
                .map(|Json(request)| request.keep_worktree)
                .unwrap_or(false),
        )
        .await
        .map_err(ApiError::close)?;
    Ok(Json(Success::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn close_operations_run_on_a_blocking_worker() {
        let request_thread = std::thread::current().id();
        let (sender, receiver) = std::sync::mpsc::channel();

        let result = run_close_on_blocking_worker(move || async move {
            sender
                .send(std::thread::current().id())
                .map_err(anyhow::Error::from)?;
            Ok(())
        })
        .await;
        let ran_on_blocking_worker = receiver
            .try_recv()
            .ok()
            .is_some_and(|worker_thread| worker_thread != request_thread);

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                ran_on_blocking_worker
            ),
            (Ok(()), true),
        );
    }
}
