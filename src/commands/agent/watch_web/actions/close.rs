use std::future::Future;

use axum::Json;
use axum::extract::{Path as RoutePath, State};
use serde::Deserialize;

use super::*;

pub(super) async fn run_on_blocking_worker<T, F, Fut>(operation: F) -> Result<T>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<T>> + Send + 'static,
    T: Send + 'static,
{
    let runtime = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || runtime.block_on(operation()))
        .await
        .map_err(anyhow::Error::from)?
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct CloseRequest {
    keep_worktree: bool,
    stop_and_close: bool,
}

pub(super) async fn close(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
    request: Option<Json<CloseRequest>>,
) -> Result<Json<Success>, ApiError> {
    let session = load_session(state.clone(), id).await?;
    let request = request.map(|Json(request)| request).unwrap_or_default();
    if let Err(error) = state
        .runtime
        .close_session(&session, request.keep_worktree, request.stop_and_close)
        .await
    {
        log_close_failure(&session.session_id, &error);
        let mut api_error = ApiError::close(error);
        if api_error.needs_close_checks() {
            let checks = (state.close_checks_inspector)(session.cwd.clone()).await;
            api_error.set_close_checks(checks);
        }
        return Err(api_error);
    }
    Ok(Json(Success::new()))
}

fn log_close_failure(session_id: &str, error: &anyhow::Error) {
    tracing::warn!(
        event = "agent.watch_web.close_failed",
        session = %session_id,
        error = %error,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_failures_are_logged_with_the_session_id_and_error() {
        let error = anyhow::anyhow!("synthetic close failure");
        let (_, events) = crate::shared::testing::capture_json_events(tracing::Level::WARN, || {
            log_close_failure("synthetic-session", &error)
        });

        assert_eq!(
            events,
            vec![serde_json::json!({
                "event": "agent.watch_web.close_failed",
                "session": "synthetic-session",
                "error": "synthetic close failure",
            })],
        );
    }

    #[tokio::test]
    async fn operations_run_on_a_blocking_worker() {
        let request_thread = std::thread::current().id();
        let (sender, receiver) = std::sync::mpsc::channel();

        let result = run_on_blocking_worker(move || async move {
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
