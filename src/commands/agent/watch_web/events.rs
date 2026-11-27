use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use futures_util::stream::{self, Stream, StreamExt};
use notify::{RecursiveMode, Watcher};
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::sync::watch;
use tokio::time;

use super::{EnrichmentUpdates, SERVER_INSTANCE_ID, SERVER_VERSION};
use crate::commands::agent::session_store_event::is_session_store_change;
use crate::commands::agent::store;

const REVISION_POLL_INTERVAL: Duration = Duration::from_secs(2);
const WATCHER_DEBOUNCE: Duration = Duration::from_millis(150);
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Clone)]
struct EventUpdates {
    enrichment: EnrichmentUpdates,
    revisions: RevisionMonitor,
    restart: watch::Receiver<bool>,
}

pub(super) fn router(updates: EnrichmentUpdates, restart: watch::Receiver<bool>) -> Router {
    Router::new()
        .route("/events", get(events))
        .with_state(EventUpdates {
            enrichment: updates,
            revisions: RevisionMonitor::new(current_revision, create_watcher),
            restart,
        })
}

async fn events(
    State(updates): State<EventUpdates>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = event_stream(
        SERVER_INSTANCE_ID.clone(),
        SERVER_VERSION,
        updates.revisions.subscribe(),
        updates.enrichment.subscribe(),
        updates.restart,
    );

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(KEEP_ALIVE_INTERVAL)
            .text("keep-alive"),
    )
}

fn event_stream(
    instance_id: String,
    server_version: &'static str,
    revisions: RevisionSubscription,
    enrichment_updates: watch::Receiver<()>,
    restart_signal: watch::Receiver<bool>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    stream::once(async move { Ok(Event::default().event("instance").data(instance_id)) })
        .chain(stream::once(async move {
            Ok(Event::default().event("version").data(server_version))
        }))
        .chain(revision_stream(
            revisions,
            enrichment_updates,
            restart_signal,
        ))
}

type RevisionReader = Arc<dyn Fn() -> Option<i64> + Send + Sync>;
type WatcherFactory =
    Arc<dyn Fn(UnboundedSender<()>) -> Option<Box<dyn std::any::Any + Send>> + Send + Sync>;

#[derive(Clone)]
struct RevisionMonitor {
    inner: Arc<RevisionMonitorInner>,
}

struct RevisionMonitorInner {
    revisions: watch::Sender<Option<i64>>,
    subscribers: watch::Sender<usize>,
}

struct RevisionSubscription {
    revisions: watch::Receiver<Option<i64>>,
    _guard: RevisionSubscriberGuard,
}

struct RevisionSubscriberGuard {
    subscribers: watch::Sender<usize>,
}

impl Drop for RevisionSubscriberGuard {
    fn drop(&mut self) {
        self.subscribers
            .send_modify(|count| *count = count.saturating_sub(1));
    }
}

impl RevisionMonitor {
    fn new(
        read_revision: impl Fn() -> Option<i64> + Send + Sync + 'static,
        create_watcher: impl Fn(UnboundedSender<()>) -> Option<Box<dyn std::any::Any + Send>>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        let (revisions, _) = watch::channel(None);
        let (subscribers, subscriber_receiver) = watch::channel(0);
        let read_revision: RevisionReader = Arc::new(read_revision);
        let create_watcher: WatcherFactory = Arc::new(create_watcher);
        tokio::spawn(run_revision_monitor(
            subscriber_receiver,
            revisions.clone(),
            Arc::clone(&read_revision),
            Arc::clone(&create_watcher),
        ));
        Self {
            inner: Arc::new(RevisionMonitorInner {
                revisions,
                subscribers,
            }),
        }
    }

    fn subscribe(&self) -> RevisionSubscription {
        self.inner.subscribers.send_modify(|count| {
            if *count == 0 {
                self.inner.revisions.send_replace(None);
            }
            *count = count.saturating_add(1);
        });
        let revisions = self.inner.revisions.subscribe();
        RevisionSubscription {
            revisions,
            _guard: RevisionSubscriberGuard {
                subscribers: self.inner.subscribers.clone(),
            },
        }
    }
}

async fn run_revision_monitor(
    mut subscriber_receiver: watch::Receiver<usize>,
    revision_sender: watch::Sender<Option<i64>>,
    read_revision: RevisionReader,
    create_watcher: WatcherFactory,
) {
    loop {
        while *subscriber_receiver.borrow() == 0 {
            if subscriber_receiver.changed().await.is_err() {
                return;
            }
        }

        let (signal_sender, mut signals) = mpsc::unbounded_channel();
        let create_watcher = Arc::clone(&create_watcher);
        let watcher = tokio::task::spawn_blocking(move || create_watcher(signal_sender))
            .await
            .ok()
            .flatten();
        if *subscriber_receiver.borrow() == 0 {
            continue;
        }
        let mut interval = time::interval(REVISION_POLL_INTERVAL);
        interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
        interval.tick().await;
        if *subscriber_receiver.borrow() > 0 {
            publish_revision(&revision_sender, &read_revision).await;
        }

        let mut watcher_channel_open = true;
        loop {
            if *subscriber_receiver.borrow() == 0 {
                break;
            }
            let _keep_watcher_alive = watcher.as_ref();
            tokio::select! {
                changed = subscriber_receiver.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    if *subscriber_receiver.borrow() == 0 {
                        break;
                    }
                }
                _ = interval.tick() => {
                    if *subscriber_receiver.borrow() > 0 {
                        publish_revision(&revision_sender, &read_revision).await;
                    }
                }
                signal = signals.recv(), if watcher_channel_open => {
                    if signal.is_none() {
                        watcher_channel_open = false;
                    } else {
                        time::sleep(WATCHER_DEBOUNCE).await;
                        while signals.try_recv().is_ok() {}
                        if *subscriber_receiver.borrow() > 0 {
                            publish_revision(&revision_sender, &read_revision).await;
                        }
                    }
                }
            }
        }
    }
}

async fn publish_revision(
    revision_sender: &watch::Sender<Option<i64>>,
    read_revision: &RevisionReader,
) {
    let read_revision = Arc::clone(read_revision);
    let Ok(Some(revision)) = tokio::task::spawn_blocking(move || read_revision()).await else {
        return;
    };
    if *revision_sender.borrow() != Some(revision) {
        revision_sender.send_replace(Some(revision));
    }
}

fn current_revision() -> Option<i64> {
    store::session_store_revision().ok()
}

fn create_watcher(signal_tx: UnboundedSender<()>) -> Option<Box<dyn std::any::Any + Send>> {
    let sessions_dir = store::sessions_dir().ok()?;
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok_and(|event| is_session_store_change(&event)) {
            let _ = signal_tx.send(());
        }
    })
    .ok()?;
    watcher
        .watch(&sessions_dir, RecursiveMode::NonRecursive)
        .ok()?;
    Some(Box::new(watcher))
}

struct RevisionStreamState {
    last_revision: Option<i64>,
    initial_event_pending: bool,
    enrichment_update_pending: bool,
    revisions: watch::Receiver<Option<i64>>,
    _subscriber_guard: RevisionSubscriberGuard,
    revision_channel_open: bool,
    enrichment_updates: watch::Receiver<()>,
    enrichment_channel_open: bool,
    restart_signal: watch::Receiver<bool>,
    restart_channel_open: bool,
    restart_event_pending: bool,
    restart_event_sent: bool,
}

fn revision_stream(
    revisions: RevisionSubscription,
    enrichment_updates: watch::Receiver<()>,
    restart_signal: watch::Receiver<bool>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    stream::unfold(
        RevisionStreamState {
            last_revision: None,
            initial_event_pending: true,
            enrichment_update_pending: false,
            revisions: revisions.revisions,
            _subscriber_guard: revisions._guard,
            revision_channel_open: true,
            enrichment_updates,
            enrichment_channel_open: true,
            restart_signal,
            restart_channel_open: true,
            restart_event_pending: false,
            restart_event_sent: false,
        },
        |mut state| async move {
            if *state.restart_signal.borrow() {
                state.restart_event_pending = true;
            }
            if state.restart_event_sent {
                return None;
            }
            if state.restart_event_pending {
                state.restart_event_pending = false;
                state.restart_event_sent = true;
                return Some((
                    Ok(Event::default().event("restart").data("restarting")),
                    state,
                ));
            }

            if state.initial_event_pending {
                let initial_revision = *state.revisions.borrow_and_update();
                if let Some(revision) = initial_revision {
                    state.initial_event_pending = false;
                    state.last_revision = Some(revision);
                    return Some((
                        Ok(Event::default()
                            .event("sessions")
                            .data(revision.to_string())),
                        state,
                    ));
                }
            }

            loop {
                let (revision_update, enrichment_update, restart_received) = tokio::select! {
                    update = state.revisions.changed(), if state.revision_channel_open => {
                        if update.is_err() {
                            state.revision_channel_open = false;
                        }
                        (update.is_ok(), false, false)
                    }
                    update = state.enrichment_updates.changed(), if state.enrichment_channel_open => {
                        if update.is_err() {
                            state.enrichment_channel_open = false;
                        }
                        (false, update.is_ok(), false)
                    }
                    update = state.restart_signal.changed(), if state.restart_channel_open => {
                        if update.is_err() {
                            state.restart_channel_open = false;
                            (false, false, false)
                        } else {
                            (false, false, *state.restart_signal.borrow())
                        }
                    }
                };
                if restart_received {
                    state.restart_event_pending = true;
                    continue;
                }
                state.enrichment_update_pending |= enrichment_update;

                let revision = if revision_update {
                    *state.revisions.borrow_and_update()
                } else {
                    *state.revisions.borrow()
                };
                let Some(revision) = revision else {
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
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

    #[tokio::test]
    async fn sends_the_server_instance_before_session_revisions() {
        let revisions = RevisionMonitor::new(|| Some(12), |_| Some(Box::new(())));
        let (_enrichment_tx, enrichment_rx) = watch::channel(());
        let (_restart_tx, restart_rx) = watch::channel(false);
        let body = Sse::new(event_stream(
            "fixture-server-instance".to_string(),
            "9.8.7",
            revisions.subscribe(),
            enrichment_rx,
            restart_rx,
        ))
        .into_response()
        .into_body();
        let mut chunks = body.into_data_stream();
        let instance = next_chunk(&mut chunks).await;
        let version = next_chunk(&mut chunks).await;
        let sessions = next_chunk(&mut chunks).await;

        assert_eq!(
            (instance, version, sessions),
            (
                indoc! {"
                    event: instance
                    data: fixture-server-instance

                "}
                .to_string(),
                indoc! {"
                    event: version
                    data: 9.8.7

                "}
                .to_string(),
                indoc! {"
                    event: sessions
                    data: 12

                "}
                .to_string(),
            ),
        );
    }

    #[tokio::test]
    async fn shares_revision_reads_and_events_across_active_connections() {
        let revision = Arc::new(AtomicI64::new(7));
        let read_count = Arc::new(AtomicUsize::new(0));
        let watcher_sender = Arc::new(std::sync::Mutex::new(None));
        let monitor = RevisionMonitor::new(
            {
                let revision = Arc::clone(&revision);
                let read_count = Arc::clone(&read_count);
                move || {
                    read_count.fetch_add(1, Ordering::SeqCst);
                    Some(revision.load(Ordering::SeqCst))
                }
            },
            {
                let watcher_sender = Arc::clone(&watcher_sender);
                move |signal_sender| {
                    *watcher_sender.lock().unwrap() = Some(signal_sender);
                    Some(Box::new(()))
                }
            },
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
        let reads_without_subscribers = read_count.load(Ordering::SeqCst);
        let (first_enrichment_sender, first_enrichment_updates) = watch::channel(());
        let (_second_enrichment_sender, second_enrichment_updates) = watch::channel(());
        let mut first = Sse::new(test_revision_stream(
            monitor.subscribe(),
            first_enrichment_updates,
        ))
        .into_response()
        .into_body()
        .into_data_stream();
        let mut second = Sse::new(test_revision_stream(
            monitor.subscribe(),
            second_enrichment_updates,
        ))
        .into_response()
        .into_body()
        .into_data_stream();
        let first_initial = next_chunk(&mut first).await;
        let second_initial = next_chunk(&mut second).await;
        let initial_read_count = read_count.load(Ordering::SeqCst);

        revision.store(8, Ordering::SeqCst);
        let signal_sender = watcher_sender.lock().unwrap().clone().unwrap();
        for _ in 0..10 {
            signal_sender.send(()).unwrap();
        }
        let first_update = next_chunk(&mut first).await;
        let second_update = next_chunk(&mut second).await;
        let update_read_count = read_count.load(Ordering::SeqCst);
        first_enrichment_sender.send_replace(());
        let enrichment_update = next_chunk(&mut first).await;
        let enrichment_read_count = read_count.load(Ordering::SeqCst);

        drop(first);
        drop(second);
        tokio::time::timeout(Duration::from_secs(1), async {
            while signal_sender.send(()).is_ok() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        tokio::time::sleep(REVISION_POLL_INTERVAL + Duration::from_millis(50)).await;
        let idle_read_count = read_count.load(Ordering::SeqCst);

        revision.store(9, Ordering::SeqCst);
        let mut reconnected = Sse::new(test_revision_stream(
            monitor.subscribe(),
            watch::channel(()).1,
        ))
        .into_response()
        .into_body()
        .into_data_stream();
        let reconnect_event = next_chunk(&mut reconnected).await;
        let reconnect_read_count = read_count.load(Ordering::SeqCst);

        assert_eq!(
            (
                reads_without_subscribers,
                first_initial,
                second_initial,
                initial_read_count,
                first_update,
                second_update,
                update_read_count,
                enrichment_update,
                enrichment_read_count,
                idle_read_count,
                reconnect_event,
                reconnect_read_count,
            ),
            (
                0,
                sessions_event(7),
                sessions_event(7),
                1,
                sessions_event(8),
                sessions_event(8),
                2,
                sessions_event(8),
                2,
                2,
                sessions_event(9),
                3,
            ),
        );
    }

    #[tokio::test]
    async fn emits_restart_and_closes_the_event_stream() {
        let revisions = RevisionMonitor::new(|| Some(12), |_| Some(Box::new(())));
        let (_enrichment_tx, enrichment_rx) = watch::channel(());
        let (restart_tx, restart_rx) = watch::channel(false);
        let stream = revision_stream(revisions.subscribe(), enrichment_rx, restart_rx);
        let body = Sse::new(stream).into_response().into_body();
        let mut chunks = body.into_data_stream();
        let initial = next_chunk(&mut chunks).await;

        restart_tx.send_replace(true);
        let restarting = next_chunk(&mut chunks).await;
        let closed = tokio::time::timeout(Duration::from_secs(1), chunks.next())
            .await
            .unwrap()
            .is_none();

        assert_eq!(
            (initial, restarting, closed),
            (
                indoc! {"
                    event: sessions
                    data: 12

                "}
                .to_string(),
                indoc! {"
                    event: restart
                    data: restarting

                "}
                .to_string(),
                true,
            ),
        );
    }

    fn test_revision_stream(
        revisions: RevisionSubscription,
        enrichment_updates: watch::Receiver<()>,
    ) -> impl Stream<Item = Result<Event, Infallible>> {
        let (_restart_sender, restart_signal) = watch::channel(false);
        revision_stream(revisions, enrichment_updates, restart_signal)
    }

    fn sessions_event(revision: i64) -> String {
        let mut event = indoc::formatdoc! {"
            event: sessions
            data: {revision}
        "};
        event.push('\n');
        event
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
