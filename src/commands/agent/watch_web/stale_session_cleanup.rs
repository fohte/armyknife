use anyhow::{Context, Result};
use tokio::task::JoinHandle;

use crate::commands::agent::store;

pub(super) fn start() -> JoinHandle<()> {
    start_with_cleanup(store::cleanup_stale_sessions)
}

fn start_with_cleanup(cleanup: impl FnOnce() -> Result<bool> + Send + 'static) -> JoinHandle<()> {
    tokio::spawn(async move {
        let result = tokio::task::spawn_blocking(cleanup)
            .await
            .context("failed to join stale session cleanup")
            .and_then(|result| result);
        if let Err(error) = result {
            tracing::warn!(%error, "failed to clean up stale agent sessions");
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[rstest::rstest]
    #[case::cleanup_succeeds(false)]
    #[case::cleanup_fails(true)]
    #[tokio::test]
    async fn runs_cleanup_once_and_finishes(#[case] should_fail: bool) {
        let cleanup_calls = Arc::new(AtomicUsize::new(0));
        let cleanup_call_counter = Arc::clone(&cleanup_calls);
        let manager = start_with_cleanup(move || {
            cleanup_call_counter.fetch_add(1, Ordering::SeqCst);
            if should_fail {
                Err(anyhow::anyhow!("fixture cleanup failure"))
            } else {
                Ok(false)
            }
        });
        let task_finished = tokio::time::timeout(Duration::from_secs(1), manager)
            .await
            .is_ok_and(|result| result.is_ok());

        assert_eq!(
            (cleanup_calls.load(Ordering::SeqCst), task_finished),
            (1, true),
        );
    }
}
