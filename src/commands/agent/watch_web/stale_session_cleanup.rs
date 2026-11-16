use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time;

use crate::commands::agent::store;

const CLEANUP_INTERVAL: Duration = Duration::from_secs(30);

type Cleanup = dyn Fn() -> Result<bool> + Send + Sync;

pub(super) fn start(shutdown: watch::Receiver<bool>) -> JoinHandle<()> {
    start_with_cleanup(shutdown, CLEANUP_INTERVAL, store::cleanup_stale_sessions)
}

fn start_with_cleanup(
    shutdown: watch::Receiver<bool>,
    interval: Duration,
    cleanup: impl Fn() -> Result<bool> + Send + Sync + 'static,
) -> JoinHandle<()> {
    tokio::spawn(run_periodic(shutdown, interval, Arc::new(cleanup)))
}

async fn run_periodic(
    mut shutdown: watch::Receiver<bool>,
    interval_duration: Duration,
    cleanup: Arc<Cleanup>,
) {
    let mut interval = time::interval(interval_duration);
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
    interval.tick().await;
    if !*shutdown.borrow() {
        run_cleanup(Arc::clone(&cleanup)).await;
    }

    loop {
        tokio::select! {
            biased;
            result = shutdown.changed() => {
                if result.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            _ = interval.tick() => run_cleanup(Arc::clone(&cleanup)).await,
        }
    }
}

async fn run_cleanup(cleanup: Arc<Cleanup>) {
    let result = tokio::task::spawn_blocking(move || cleanup())
        .await
        .context("failed to join stale session cleanup")
        .and_then(|result| result);
    if let Err(error) = result {
        tracing::warn!(%error, "failed to clean up stale agent sessions");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::time::timeout;

    #[tokio::test]
    async fn runs_cleanup_at_start_and_retries_after_an_error_until_shutdown() {
        let (shutdown_sender, shutdown_receiver) = watch::channel(false);
        let (cleanup_sender, mut cleanup_receiver) = tokio::sync::mpsc::unbounded_channel();
        let cleanup_calls = Arc::new(AtomicUsize::new(0));
        let cleanup_call_counter = Arc::clone(&cleanup_calls);
        let cleanup_shutdown_sender = shutdown_sender.clone();
        let manager = start_with_cleanup(shutdown_receiver, Duration::from_millis(10), move || {
            let call = cleanup_call_counter.fetch_add(1, Ordering::SeqCst) + 1;
            let _ = cleanup_sender.send(());
            if call == 2 {
                return Err(anyhow::anyhow!("fixture cleanup failure"));
            }
            if call == 3 {
                cleanup_shutdown_sender.send_replace(true);
            }
            Ok(false)
        });

        for _ in 0..3 {
            timeout(Duration::from_secs(1), cleanup_receiver.recv())
                .await
                .expect("cleanup should run")
                .expect("cleanup task should remain active");
        }
        timeout(Duration::from_secs(1), manager)
            .await
            .expect("cleanup task should stop")
            .expect("cleanup task should not panic");

        assert_eq!(
            (cleanup_calls.load(Ordering::SeqCst), CLEANUP_INTERVAL,),
            (3, Duration::from_secs(30)),
        );
    }
}
