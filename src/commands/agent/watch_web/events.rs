use std::convert::Infallible;
use std::future::Future;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use futures_util::stream::{self, Stream};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc::{self, UnboundedReceiver};
use tokio::sync::watch;
use tokio::time::{self, Interval};

use super::EnrichmentUpdates;
use crate::commands::agent::store;

const REVISION_POLL_INTERVAL: Duration = Duration::from_secs(2);
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(15);

pub(super) fn router(updates: EnrichmentUpdates) -> Router {
    Router::new()
        .route("/events", get(events))
        .with_state(updates)
}

async fn events(
    State(updates): State<EnrichmentUpdates>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (signal_tx, signal_rx) = mpsc::unbounded_channel();
    let watcher = create_watcher(signal_tx).await;
    let stream = revision_stream(signal_rx, updates.subscribe(), watcher, current_revision);

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(KEEP_ALIVE_INTERVAL)
            .text("keep-alive"),
    )
}

async fn create_watcher(
    signal_tx: tokio::sync::mpsc::UnboundedSender<()>,
) -> Option<RecommendedWatcher> {
    tokio::task::spawn_blocking(move || {
        let sessions_dir = store::sessions_dir()?;
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if event.is_ok() {
                    let _ = signal_tx.send(());
                }
            })?;
        watcher.watch(&sessions_dir, RecursiveMode::NonRecursive)?;
        Ok::<_, anyhow::Error>(watcher)
    })
    .await
    .ok()
    .and_then(Result::ok)
}

async fn current_revision() -> Option<i64> {
    tokio::task::spawn_blocking(store::session_store_revision)
        .await
        .ok()
        .and_then(Result::ok)
}

struct RevisionStreamState<F> {
    last_revision: Option<i64>,
    initial_event_pending: bool,
    enrichment_update_pending: bool,
    signals: UnboundedReceiver<()>,
    watcher: Option<RecommendedWatcher>,
    watch_channel_open: bool,
    enrichment_updates: watch::Receiver<()>,
    enrichment_channel_open: bool,
    interval: Interval,
    read_revision: F,
}

fn revision_stream<F, Fut>(
    signals: UnboundedReceiver<()>,
    enrichment_updates: watch::Receiver<()>,
    watcher: Option<RecommendedWatcher>,
    read_revision: F,
) -> impl Stream<Item = Result<Event, Infallible>>
where
    F: Fn() -> Fut + Send + 'static,
    Fut: Future<Output = Option<i64>> + Send + 'static,
{
    let mut interval = time::interval(REVISION_POLL_INTERVAL);
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
    stream::unfold(
        RevisionStreamState {
            last_revision: None,
            initial_event_pending: true,
            enrichment_update_pending: false,
            signals,
            watcher,
            watch_channel_open: true,
            enrichment_updates,
            enrichment_channel_open: true,
            interval,
            read_revision,
        },
        |mut state| async move {
            loop {
                let _keep_watcher_alive = state.watcher.as_ref();
                let enrichment_update = tokio::select! {
                    _ = state.interval.tick() => false,
                    signal = state.signals.recv(), if state.watch_channel_open => {
                        if signal.is_none() {
                            state.watch_channel_open = false;
                        }
                        false
                    }
                    update = state.enrichment_updates.changed(), if state.enrichment_channel_open => {
                        if update.is_err() {
                            state.enrichment_channel_open = false;
                        }
                        update.is_ok()
                    }
                };
                state.enrichment_update_pending |= enrichment_update;

                let Some(revision) = (state.read_revision)().await else {
                    continue;
                };
                let should_emit = state.initial_event_pending
                    || state.last_revision != Some(revision)
                    || state.enrichment_update_pending;
                state.initial_event_pending = false;
                state.enrichment_update_pending = false;
                state.last_revision = Some(revision);
                if !should_emit {
                    continue;
                }

                return Some((
                    Ok(Event::default()
                        .event("sessions")
                        .data(revision.to_string())),
                    state,
                ));
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;
    use futures_util::StreamExt;
    use indoc::indoc;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, Ordering};

    #[tokio::test]
    async fn sends_current_revision_on_connect_and_after_store_or_enrichment_changes() {
        let revision = Arc::new(AtomicI64::new(7));
        let (signal_tx, signal_rx) = mpsc::unbounded_channel();
        let (enrichment_tx, enrichment_rx) = watch::channel(());
        let read_revision = {
            let revision = Arc::clone(&revision);
            move || {
                let revision = Arc::clone(&revision);
                async move { Some(revision.load(Ordering::SeqCst)) }
            }
        };
        let body = Sse::new(revision_stream(
            signal_rx,
            enrichment_rx,
            None,
            read_revision,
        ))
        .into_response()
        .into_body();
        let mut chunks = body.into_data_stream();
        let first = next_chunk(&mut chunks).await;

        revision.store(8, Ordering::SeqCst);
        signal_tx.send(()).unwrap();
        let second = next_chunk(&mut chunks).await;

        enrichment_tx.send_replace(());
        let enrichment_update = next_chunk(&mut chunks).await;

        let (_reconnect_signal_tx, reconnect_signal_rx) = mpsc::unbounded_channel();
        let (_reconnect_enrichment_tx, reconnect_enrichment_rx) = watch::channel(());
        let read_current_revision = {
            let revision = Arc::clone(&revision);
            move || {
                let revision = Arc::clone(&revision);
                async move { Some(revision.load(Ordering::SeqCst)) }
            }
        };
        let reconnect_body = Sse::new(revision_stream(
            reconnect_signal_rx,
            reconnect_enrichment_rx,
            None,
            read_current_revision,
        ))
        .into_response()
        .into_body();
        let mut reconnect_chunks = reconnect_body.into_data_stream();
        let reconnected = next_chunk(&mut reconnect_chunks).await;

        assert_eq!(
            (first, second, enrichment_update, reconnected),
            (
                indoc! {"
                    event: sessions
                    data: 7

                "}
                .to_string(),
                indoc! {"
                    event: sessions
                    data: 8

                "}
                .to_string(),
                indoc! {"
                    event: sessions
                    data: 8

                "}
                .to_string(),
                indoc! {"
                    event: sessions
                    data: 8

                "}
                .to_string(),
            ),
        );
    }

    async fn next_chunk(
        chunks: &mut (impl Stream<Item = Result<axum::body::Bytes, axum::Error>> + Unpin),
    ) -> String {
        let chunk = tokio::time::timeout(Duration::from_secs(1), chunks.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        String::from_utf8(chunk.to_vec()).unwrap()
    }
}
