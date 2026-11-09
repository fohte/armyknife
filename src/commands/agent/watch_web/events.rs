use std::convert::Infallible;
use std::time::Duration;

use axum::Router;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use futures_util::stream::{self, Stream};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc::{self, UnboundedReceiver};
use tokio::time::{self, Interval};

use crate::commands::agent::store;

const REVISION_POLL_INTERVAL: Duration = Duration::from_secs(2);
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(15);

pub(super) fn router() -> Router {
    Router::new().route("/events", get(events))
}

async fn events() -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (signal_tx, signal_rx) = mpsc::unbounded_channel();
    let watcher = create_watcher(signal_tx).await;
    let revision = current_revision().await;
    let stream = revision_stream(revision, signal_rx, watcher);

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

struct RevisionStreamState {
    last_revision: Option<i64>,
    signals: UnboundedReceiver<()>,
    watcher: Option<RecommendedWatcher>,
    watch_channel_open: bool,
    interval: Interval,
}

fn revision_stream(
    last_revision: Option<i64>,
    signals: UnboundedReceiver<()>,
    watcher: Option<RecommendedWatcher>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    let mut interval = time::interval(REVISION_POLL_INTERVAL);
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
    stream::unfold(
        RevisionStreamState {
            last_revision,
            signals,
            watcher,
            watch_channel_open: true,
            interval,
        },
        |mut state| async move {
            loop {
                let _keep_watcher_alive = state.watcher.as_ref();
                tokio::select! {
                    _ = state.interval.tick() => {}
                    signal = state.signals.recv(), if state.watch_channel_open => {
                        if signal.is_none() {
                            state.watch_channel_open = false;
                        }
                    }
                }

                let Some(revision) = current_revision().await else {
                    continue;
                };
                if state.last_revision == Some(revision) {
                    continue;
                }

                state.last_revision = Some(revision);
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
