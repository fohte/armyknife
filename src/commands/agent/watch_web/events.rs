use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use futures_util::stream::{self, Stream, StreamExt};
use tokio::sync::{broadcast, watch};
use tokio::time;

use super::{EnrichmentUpdates, SERVER_INSTANCE_ID, SERVER_VERSION, usage};
use crate::commands::agent::store;

const WATCHER_DEBOUNCE: Duration = Duration::from_millis(150);
const REVISION_RETRY_INITIAL_DELAY: Duration = Duration::from_millis(200);
const REVISION_RETRY_MAX_ATTEMPTS: usize = 4;
const REVISION_RETRY_WARNING_INTERVAL: Duration = Duration::from_secs(30);
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Clone)]
struct EventUpdates {
    enrichment: EnrichmentUpdates,
    revisions: RevisionMonitor,
    usage: usage::UsageMonitor,
    restart: watch::Receiver<bool>,
}

pub(super) fn router(
    updates: EnrichmentUpdates,
    restart: watch::Receiver<bool>,
    session_store_events: broadcast::Receiver<()>,
    session_store_watcher_available: bool,
) -> Router {
    let usage =
        usage::UsageMonitor::new(usage::LiveUsageSource, session_store_events.resubscribe());
    Router::new()
        .route("/events", get(events))
        .with_state(EventUpdates {
            enrichment: updates,
            revisions: RevisionMonitor::new(
                current_revision,
                session_store_events,
                session_store_watcher_available,
            ),
            usage,
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
        updates.usage.subscribe(),
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
    usage: usage::UsageSubscription,
    enrichment_updates: watch::Receiver<()>,
    restart_signal: watch::Receiver<bool>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    stream::once(async move { Ok(Event::default().event("instance").data(instance_id)) })
        .chain(stream::once(async move {
            Ok(Event::default().event("version").data(server_version))
        }))
        .chain(revision_stream(
            revisions,
            usage,
            enrichment_updates,
            restart_signal,
        ))
}

type RevisionReader = Arc<dyn Fn() -> anyhow::Result<i64> + Send + Sync>;

#[derive(Clone)]
struct RevisionMonitor {
    inner: Arc<RevisionMonitorInner>,
}

struct RevisionMonitorInner {
    revisions: watch::Sender<Option<i64>>,
    subscribers: watch::Sender<usize>,
    session_store_watcher_available: watch::Sender<bool>,
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
        read_revision: impl Fn() -> anyhow::Result<i64> + Send + Sync + 'static,
        session_store_events: broadcast::Receiver<()>,
        session_store_watcher_available: bool,
    ) -> Self {
        let (revisions, _) = watch::channel(None);
        let (subscribers, subscriber_receiver) = watch::channel(0);
        let (session_store_watcher_available, _) = watch::channel(session_store_watcher_available);
        let read_revision: RevisionReader = Arc::new(read_revision);
        tokio::spawn(run_revision_monitor(
            subscriber_receiver,
            revisions.clone(),
            Arc::clone(&read_revision),
            session_store_events,
            session_store_watcher_available.clone(),
        ));
        Self {
            inner: Arc::new(RevisionMonitorInner {
                revisions,
                subscribers,
                session_store_watcher_available,
            }),
        }
    }

    fn subscribe(&self) -> RevisionSubscription {
        self.inner.subscribers.send_modify(|count| {
            if *count == 0 || !*self.inner.session_store_watcher_available.borrow() {
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
    mut session_store_events: broadcast::Receiver<()>,
    session_store_watcher_available: watch::Sender<bool>,
) {
    let mut event_channel_open = true;
    let mut last_warning_at = None;
    let mut refresh_pending;
    loop {
        while *subscriber_receiver.borrow_and_update() == 0 {
            if event_channel_open {
                tokio::select! {
                    changed = subscriber_receiver.changed() => {
                        if changed.is_err() {
                            return;
                        }
                    }
                    signal = session_store_events.recv() => {
                        if matches!(signal, Err(broadcast::error::RecvError::Closed)) {
                            event_channel_open = false;
                        }
                    }
                }
            } else if subscriber_receiver.changed().await.is_err() {
                return;
            }
        }

        match publish_revision_with_retries(
            &mut subscriber_receiver,
            &revision_sender,
            &read_revision,
            &mut last_warning_at,
        )
        .await
        {
            PublishRevisionOutcome::Published => refresh_pending = false,
            PublishRevisionOutcome::NoSubscribers => continue,
            PublishRevisionOutcome::RetriesExhausted => refresh_pending = true,
        }

        loop {
            if *subscriber_receiver.borrow() == 0 {
                break;
            }
            tokio::select! {
                changed = subscriber_receiver.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    if *subscriber_receiver.borrow() == 0 {
                        break;
                    }
                    if refresh_pending
                        || revision_sender.borrow().is_none()
                        || !*session_store_watcher_available.borrow()
                    {
                        match publish_revision_with_retries(
                            &mut subscriber_receiver,
                            &revision_sender,
                            &read_revision,
                            &mut last_warning_at,
                        )
                        .await
                    {
                            PublishRevisionOutcome::Published => refresh_pending = false,
                            PublishRevisionOutcome::NoSubscribers => break,
                            PublishRevisionOutcome::RetriesExhausted => refresh_pending = true,
                        }
                    }
                }
                signal = session_store_events.recv(), if event_channel_open => {
                    if matches!(signal, Err(broadcast::error::RecvError::Closed)) {
                        event_channel_open = false;
                        session_store_watcher_available.send_replace(false);
                    } else {
                        time::sleep(WATCHER_DEBOUNCE).await;
                        while matches!(
                            session_store_events.try_recv(),
                            Ok(()) | Err(broadcast::error::TryRecvError::Lagged(_))
                        ) {}
                        if *subscriber_receiver.borrow() > 0 {
                            match publish_revision_with_retries(
                                &mut subscriber_receiver,
                                &revision_sender,
                                &read_revision,
                                &mut last_warning_at,
                            )
                            .await
                            {
                                PublishRevisionOutcome::Published => refresh_pending = false,
                                PublishRevisionOutcome::NoSubscribers => break,
                                PublishRevisionOutcome::RetriesExhausted => refresh_pending = true,
                            }
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
) -> anyhow::Result<()> {
    let read_revision = Arc::clone(read_revision);
    let revision = tokio::task::spawn_blocking(move || read_revision()).await??;
    if *revision_sender.borrow() != Some(revision) {
        revision_sender.send_replace(Some(revision));
    }
    Ok(())
}

enum PublishRevisionOutcome {
    Published,
    NoSubscribers,
    RetriesExhausted,
}

async fn publish_revision_with_retries(
    subscriber_receiver: &mut watch::Receiver<usize>,
    revision_sender: &watch::Sender<Option<i64>>,
    read_revision: &RevisionReader,
    last_warning_at: &mut Option<time::Instant>,
) -> PublishRevisionOutcome {
    let mut retry_delay = REVISION_RETRY_INITIAL_DELAY;
    for attempt in 1..=REVISION_RETRY_MAX_ATTEMPTS {
        if *subscriber_receiver.borrow() == 0 {
            return PublishRevisionOutcome::NoSubscribers;
        }
        match publish_revision(revision_sender, read_revision).await {
            Ok(()) => return PublishRevisionOutcome::Published,
            Err(error)
                if last_warning_at
                    .is_none_or(|last| last.elapsed() >= REVISION_RETRY_WARNING_INTERVAL) =>
            {
                tracing::warn!(%error, "failed to read session store revision");
                *last_warning_at = Some(time::Instant::now());
            }
            Err(_) => {}
        }
        if attempt == REVISION_RETRY_MAX_ATTEMPTS {
            return PublishRevisionOutcome::RetriesExhausted;
        }

        let sleep = time::sleep(retry_delay);
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                () = &mut sleep => break,
                changed = subscriber_receiver.changed() => {
                    if changed.is_err() || *subscriber_receiver.borrow() == 0 {
                        return PublishRevisionOutcome::NoSubscribers;
                    }
                }
            }
        }
        retry_delay = retry_delay.saturating_mul(2);
    }
    PublishRevisionOutcome::RetriesExhausted
}

fn current_revision() -> anyhow::Result<i64> {
    store::session_store_revision()
}

struct RevisionStreamState {
    last_revision: Option<i64>,
    initial_event_pending: bool,
    enrichment_update_pending: bool,
    revisions: watch::Receiver<Option<i64>>,
    _subscriber_guard: RevisionSubscriberGuard,
    usage_updates: watch::Receiver<
        Option<Result<crate::commands::agent::usage::model::UsageResponse, String>>,
    >,
    _usage_subscriber_guard: usage::UsageSubscriberGuard,
    initial_usage_event_pending: bool,
    usage_channel_open: bool,
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
    usage_subscription: usage::UsageSubscription,
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
            usage_updates: usage_subscription.updates,
            _usage_subscriber_guard: usage_subscription._guard,
            initial_usage_event_pending: true,
            usage_channel_open: true,
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

            if state.initial_usage_event_pending {
                let snapshot = { state.usage_updates.borrow_and_update().clone() };
                if let Some(snapshot) = snapshot {
                    state.initial_usage_event_pending = false;
                    return Some((Ok(usage::usage_event(snapshot)), state));
                }
            }

            loop {
                let (revision_update, enrichment_update, restart_received, usage_update) = tokio::select! {
                    update = state.revisions.changed(), if state.revision_channel_open => {
                        if update.is_err() {
                            state.revision_channel_open = false;
                        }
                        (update.is_ok(), false, false, false)
                    }
                    update = state.enrichment_updates.changed(), if state.enrichment_channel_open => {
                        if update.is_err() {
                            state.enrichment_channel_open = false;
                        }
                        (false, update.is_ok(), false, false)
                    }
                    update = state.restart_signal.changed(), if state.restart_channel_open => {
                        if update.is_err() {
                            state.restart_channel_open = false;
                            (false, false, false, false)
                        } else {
                            (false, false, *state.restart_signal.borrow(), false)
                        }
                    }
                    update = state.usage_updates.changed(), if state.usage_channel_open => {
                        if update.is_err() {
                            state.usage_channel_open = false;
                        }
                        (false, false, false, update.is_ok())
                    }
                };
                if restart_received {
                    state.restart_event_pending = true;
                    continue;
                }
                if usage_update {
                    let snapshot = { state.usage_updates.borrow_and_update().clone() };
                    if let Some(snapshot) = snapshot {
                        state.initial_usage_event_pending = false;
                        return Some((Ok(usage::usage_event(snapshot)), state));
                    }
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
    use crate::commands::agent::types::Engine;
    use crate::commands::agent::usage::model::{
        EngineQuotas, RateLimits, SessionUsage, UsageResponse,
    };
    use axum::response::IntoResponse;
    use futures_util::StreamExt;
    use indoc::indoc;
    use rstest::rstest;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
    use std::sync::{Mutex, mpsc};
    use tokio::sync::mpsc as tokio_mpsc;

    use super::super::session_store_watcher::SessionStoreWatcher;

    #[derive(Clone, Copy)]
    enum UsageEventCase {
        Initial,
        Changed,
        Unchanged,
        Error,
    }

    #[derive(Clone)]
    struct ScriptedUsageSource(Arc<Mutex<VecDeque<anyhow::Result<UsageResponse>>>>);

    impl usage::UsageSource for ScriptedUsageSource {
        async fn load(&self) -> anyhow::Result<UsageResponse> {
            self.0
                .lock()
                .expect("usage source lock should not be poisoned")
                .pop_front()
                .unwrap_or_else(|| Err(anyhow::anyhow!("fixture usage exhausted")))
        }
    }

    #[rstest]
    #[case::initial(UsageEventCase::Initial)]
    #[case::changed(UsageEventCase::Changed)]
    #[case::unchanged(UsageEventCase::Unchanged)]
    #[case::error(UsageEventCase::Error)]
    #[tokio::test]
    async fn event_stream_sends_usage_only_when_the_snapshot_changes(#[case] case: UsageEventCase) {
        let initial_usage = UsageResponse {
            sessions: Vec::new(),
            quotas: EngineQuotas::default(),
        };
        let changed_usage = UsageResponse {
            sessions: vec![SessionUsage {
                session_id: "fixture-session".to_owned(),
                engine: Engine::Claude,
                model: Some("fixture-model".to_owned()),
                effort: None,
                context: None,
                rate_limits: RateLimits::default(),
                version: None,
                observed_at: chrono::DateTime::parse_from_rfc3339("2026-11-16T12:00:00Z")
                    .unwrap()
                    .to_utc(),
            }],
            quotas: EngineQuotas::default(),
        };
        let snapshots = match case {
            UsageEventCase::Initial => vec![Ok(initial_usage.clone())],
            UsageEventCase::Changed => vec![Ok(initial_usage.clone()), Ok(changed_usage)],
            UsageEventCase::Unchanged => vec![Ok(initial_usage.clone()), Ok(initial_usage)],
            UsageEventCase::Error => vec![Err(anyhow::anyhow!("fixture usage failure"))],
        };
        let source = ScriptedUsageSource(Arc::new(Mutex::new(snapshots.into())));
        let (usage_event_sender, usage_event_receiver) = broadcast::channel(4);
        let (_revision_event_sender, revision_event_receiver) = broadcast::channel(4);
        let revisions = RevisionMonitor::new(|| Ok(12), revision_event_receiver, true);
        let usage_monitor = usage::UsageMonitor::new(source, usage_event_receiver);
        let mut revision_subscription = revisions.subscribe();
        while revision_subscription.revisions.borrow().is_none() {
            revision_subscription.revisions.changed().await.unwrap();
        }
        let mut usage_subscription = usage_monitor.subscribe();
        while usage_subscription.updates.borrow().is_none() {
            usage_subscription.updates.changed().await.unwrap();
        }
        let (_enrichment_sender, enrichment_updates) = watch::channel(());
        let (_restart_sender, restart_signal) = watch::channel(false);
        let mut chunks = Sse::new(event_stream(
            "fixture-server-instance".to_owned(),
            "9.8.7",
            revision_subscription,
            usage_subscription,
            enrichment_updates,
            restart_signal,
        ))
        .into_response()
        .into_body()
        .into_data_stream();
        let connection = format!(
            "{}{}{}",
            next_chunk(&mut chunks).await,
            next_chunk(&mut chunks).await,
            next_chunk(&mut chunks).await,
        );
        let initial_event = next_chunk(&mut chunks).await;
        let (actual, no_follow_up_event) = match case {
            UsageEventCase::Initial | UsageEventCase::Error => {
                (format!("{connection}{initial_event}"), None)
            }
            UsageEventCase::Changed => {
                usage_event_sender.send(()).unwrap();
                (
                    format!(
                        "{connection}{initial_event}{}",
                        next_usage_chunk(&mut chunks).await
                    ),
                    None,
                )
            }
            UsageEventCase::Unchanged => {
                usage_event_sender.send(()).unwrap();
                let no_follow_up_event =
                    tokio::time::timeout(Duration::from_millis(1250), chunks.next())
                        .await
                        .is_err();
                (
                    format!("{connection}{initial_event}"),
                    Some(no_follow_up_event),
                )
            }
        };
        let expected = match case {
            UsageEventCase::Initial | UsageEventCase::Unchanged => indoc! {"
                event: instance
                data: fixture-server-instance

                event: version
                data: 9.8.7

                event: sessions
                data: 12

                event: usage
                data: {\"sessions\":[],\"quotas\":{\"claude\":null,\"codex\":null}}

            "}
            .to_owned(),
            UsageEventCase::Changed => indoc! {"
                event: instance
                data: fixture-server-instance

                event: version
                data: 9.8.7

                event: sessions
                data: 12

                event: usage
                data: {\"sessions\":[],\"quotas\":{\"claude\":null,\"codex\":null}}

                event: usage
                data: {\"sessions\":[{\"sessionId\":\"fixture-session\",\"engine\":\"claude\",\"model\":\"fixture-model\",\"effort\":null,\"context\":null,\"rateLimits\":{\"fiveHour\":null,\"sevenDay\":null},\"version\":null,\"observedAt\":\"2026-11-16T12:00:00Z\"}],\"quotas\":{\"claude\":null,\"codex\":null}}

            "}
            .to_owned(),
            UsageEventCase::Error => indoc! {"
                event: instance
                data: fixture-server-instance

                event: version
                data: 9.8.7

                event: sessions
                data: 12

                event: usage-error
                data: fixture usage failure

            "}
            .to_owned(),
        };
        let expected_no_follow_up_event = match case {
            UsageEventCase::Unchanged => Some(true),
            UsageEventCase::Initial | UsageEventCase::Changed | UsageEventCase::Error => None,
        };

        assert_eq!(
            (actual, no_follow_up_event),
            (expected, expected_no_follow_up_event),
        );
    }

    #[tokio::test]
    async fn sends_the_server_instance_before_session_revisions() {
        let (_event_sender, event_receiver) = broadcast::channel(1);
        let revisions = RevisionMonitor::new(|| Ok(12), event_receiver, true);
        let (_enrichment_tx, enrichment_rx) = watch::channel(());
        let (_restart_tx, restart_rx) = watch::channel(false);
        let body = Sse::new(event_stream(
            "fixture-server-instance".to_string(),
            "9.8.7",
            revisions.subscribe(),
            usage::no_updates_subscription(),
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
        let (event_sender, event_receiver) = broadcast::channel(16);
        let monitor = RevisionMonitor::new(
            {
                let revision = Arc::clone(&revision);
                let read_count = Arc::clone(&read_count);
                move || {
                    read_count.fetch_add(1, Ordering::SeqCst);
                    Ok(revision.load(Ordering::SeqCst))
                }
            },
            event_receiver,
            true,
        );
        event_sender.send(()).unwrap();
        tokio::time::sleep(WATCHER_DEBOUNCE + Duration::from_millis(50)).await;
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
        for _ in 0..10 {
            event_sender.send(()).unwrap();
        }
        let first_update = next_chunk(&mut first).await;
        let second_update = next_chunk(&mut second).await;
        let update_read_count = read_count.load(Ordering::SeqCst);
        first_enrichment_sender.send_replace(());
        let enrichment_update = next_chunk(&mut first).await;
        let enrichment_read_count = read_count.load(Ordering::SeqCst);

        drop(first);
        drop(second);
        event_sender.send(()).unwrap();
        tokio::time::sleep(WATCHER_DEBOUNCE + Duration::from_millis(50)).await;
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
    async fn retries_failed_revision_reads_until_a_revision_is_published() {
        let read_count = Arc::new(AtomicUsize::new(0));
        let (_event_sender, event_receiver) = broadcast::channel(1);
        let monitor = RevisionMonitor::new(
            {
                let read_count = Arc::clone(&read_count);
                move || {
                    let attempt = read_count.fetch_add(1, Ordering::SeqCst);
                    if attempt == 0 {
                        Err(anyhow::anyhow!("temporary revision read failure"))
                    } else {
                        Ok(21)
                    }
                }
            },
            event_receiver,
            true,
        );
        let mut stream = Sse::new(test_revision_stream(
            monitor.subscribe(),
            watch::channel(()).1,
        ))
        .into_response()
        .into_body()
        .into_data_stream();

        let published_event = next_chunk(&mut stream).await;
        let reads_at_publish = read_count.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(250)).await;
        let reads_after_success = read_count.load(Ordering::SeqCst);

        assert_eq!(
            (published_event, reads_at_publish, reads_after_success),
            (sessions_event(21), 2, 2),
        );
    }

    #[tokio::test]
    async fn retries_stop_until_a_new_connection_after_repeated_read_failures() {
        let read_count = Arc::new(AtomicUsize::new(0));
        let (read_attempt_sender, mut read_attempt_receiver) = tokio_mpsc::unbounded_channel();
        let (_event_sender, event_receiver) = broadcast::channel(1);
        let monitor = RevisionMonitor::new(
            {
                let read_count = Arc::clone(&read_count);
                move || {
                    let attempt = read_count.fetch_add(1, Ordering::SeqCst) + 1;
                    read_attempt_sender.send(attempt).unwrap();
                    if attempt <= REVISION_RETRY_MAX_ATTEMPTS {
                        Err(anyhow::anyhow!("fixture revision read failure"))
                    } else {
                        Ok(23)
                    }
                }
            },
            event_receiver,
            true,
        );
        let mut first = Sse::new(test_revision_stream(
            monitor.subscribe(),
            watch::channel(()).1,
        ))
        .into_response()
        .into_body()
        .into_data_stream();

        let mut failed_attempts = Vec::new();
        for _ in 0..REVISION_RETRY_MAX_ATTEMPTS {
            failed_attempts.push(read_attempt_receiver.recv().await.unwrap());
        }
        let retry_stopped =
            tokio::time::timeout(Duration::from_secs(2), read_attempt_receiver.recv())
                .await
                .is_err();

        let mut second = Sse::new(test_revision_stream(
            monitor.subscribe(),
            watch::channel(()).1,
        ))
        .into_response()
        .into_body()
        .into_data_stream();
        let successful_attempt = read_attempt_receiver.recv().await.unwrap();
        let (first_event, second_event) =
            tokio::join!(next_chunk(&mut first), next_chunk(&mut second),);

        assert_eq!(
            (
                failed_attempts,
                retry_stopped,
                successful_attempt,
                first_event,
                second_event,
                read_count.load(Ordering::SeqCst),
            ),
            (
                vec![1, 2, 3, 4],
                true,
                5,
                sessions_event(23),
                sessions_event(23),
                5,
            ),
        );
    }

    #[tokio::test]
    async fn publishes_a_revision_after_a_rapid_reconnect_with_watcher_available() {
        let revision = Arc::new(AtomicI64::new(41));
        let read_count = Arc::new(AtomicUsize::new(0));
        let (_event_sender, event_receiver) = broadcast::channel(1);
        let monitor = RevisionMonitor::new(
            {
                let revision = Arc::clone(&revision);
                let read_count = Arc::clone(&read_count);
                move || {
                    read_count.fetch_add(1, Ordering::SeqCst);
                    Ok(revision.load(Ordering::SeqCst))
                }
            },
            event_receiver,
            true,
        );
        let mut first = Sse::new(test_revision_stream(
            monitor.subscribe(),
            watch::channel(()).1,
        ))
        .into_response()
        .into_body()
        .into_data_stream();
        let first_event = next_chunk(&mut first).await;

        drop(first);
        revision.store(42, Ordering::SeqCst);
        let mut second = Sse::new(test_revision_stream(
            monitor.subscribe(),
            watch::channel(()).1,
        ))
        .into_response()
        .into_body()
        .into_data_stream();
        let second_event = next_chunk(&mut second).await;

        assert_eq!(
            (first_event, second_event, read_count.load(Ordering::SeqCst),),
            (sessions_event(41), sessions_event(42), 2),
        );
    }

    #[tokio::test]
    async fn reads_once_per_connection_when_the_store_watcher_is_unavailable() {
        let revision = Arc::new(AtomicI64::new(41));
        let read_count = Arc::new(AtomicUsize::new(0));
        let event_receiver = SessionStoreWatcher::subscribe_or_closed(None);
        let monitor = RevisionMonitor::new(
            {
                let revision = Arc::clone(&revision);
                let read_count = Arc::clone(&read_count);
                move || {
                    read_count.fetch_add(1, Ordering::SeqCst);
                    Ok(revision.load(Ordering::SeqCst))
                }
            },
            event_receiver,
            false,
        );
        let mut first = Sse::new(test_revision_stream(
            monitor.subscribe(),
            watch::channel(()).1,
        ))
        .into_response()
        .into_body()
        .into_data_stream();
        let first_event = next_chunk(&mut first).await;

        revision.store(42, Ordering::SeqCst);
        let no_follow_up_event = tokio::time::timeout(Duration::from_millis(250), first.next())
            .await
            .is_err();
        let reads_after_first_connection = read_count.load(Ordering::SeqCst);

        let mut second = Sse::new(test_revision_stream(
            monitor.subscribe(),
            watch::channel(()).1,
        ))
        .into_response()
        .into_body()
        .into_data_stream();
        let (first_update, second_event) =
            tokio::join!(next_chunk(&mut first), next_chunk(&mut second),);
        let reads_after_second_connection = read_count.load(Ordering::SeqCst);

        assert_eq!(
            (
                first_event,
                no_follow_up_event,
                reads_after_first_connection,
                first_update,
                second_event,
                reads_after_second_connection,
            ),
            (
                sessions_event(41),
                true,
                1,
                sessions_event(42),
                sessions_event(42),
                2,
            ),
        );
    }

    #[tokio::test]
    async fn rereads_and_publishes_after_lagged_store_events() {
        let revision = Arc::new(AtomicI64::new(21));
        let read_count = Arc::new(AtomicUsize::new(0));
        let (event_sender, event_receiver) = broadcast::channel(1);
        let monitor = RevisionMonitor::new(
            {
                let revision = Arc::clone(&revision);
                let read_count = Arc::clone(&read_count);
                move || {
                    read_count.fetch_add(1, Ordering::SeqCst);
                    Ok(revision.load(Ordering::SeqCst))
                }
            },
            event_receiver,
            true,
        );
        let mut stream = Sse::new(test_revision_stream(
            monitor.subscribe(),
            watch::channel(()).1,
        ))
        .into_response()
        .into_body()
        .into_data_stream();
        let initial_event = next_chunk(&mut stream).await;

        revision.store(22, Ordering::SeqCst);
        for _ in 0..3 {
            event_sender.send(()).unwrap();
        }
        let updated_event = next_chunk(&mut stream).await;
        let read_count = read_count.load(Ordering::SeqCst);

        assert_eq!(
            (initial_event, updated_event, read_count),
            (sessions_event(21), sessions_event(22), 2),
        );
    }

    #[tokio::test]
    async fn publishes_store_events_received_during_a_revision_snapshot() {
        let revision = Arc::new(AtomicI64::new(31));
        let read_count = Arc::new(AtomicUsize::new(0));
        let (snapshot_started_sender, snapshot_started_receiver) = mpsc::channel();
        let (snapshot_continue_sender, snapshot_continue_receiver) = mpsc::channel();
        let snapshot_continue_receiver = Arc::new(Mutex::new(snapshot_continue_receiver));
        let (event_sender, event_receiver) = broadcast::channel(4);
        let monitor = RevisionMonitor::new(
            {
                let revision = Arc::clone(&revision);
                let read_count = Arc::clone(&read_count);
                let snapshot_continue_receiver = Arc::clone(&snapshot_continue_receiver);
                move || {
                    let attempt = read_count.fetch_add(1, Ordering::SeqCst) + 1;
                    let snapshot = revision.load(Ordering::SeqCst);
                    if attempt == 2 {
                        snapshot_started_sender.send(()).unwrap();
                        snapshot_continue_receiver.lock().unwrap().recv().unwrap();
                    }
                    Ok(snapshot)
                }
            },
            event_receiver,
            true,
        );
        let mut stream = Sse::new(test_revision_stream(
            monitor.subscribe(),
            watch::channel(()).1,
        ))
        .into_response()
        .into_body()
        .into_data_stream();
        let initial_event = next_chunk(&mut stream).await;

        revision.store(32, Ordering::SeqCst);
        event_sender.send(()).unwrap();
        tokio::time::timeout(
            Duration::from_secs(1),
            tokio::task::spawn_blocking(move || snapshot_started_receiver.recv()),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        revision.store(33, Ordering::SeqCst);
        event_sender.send(()).unwrap();
        snapshot_continue_sender.send(()).unwrap();

        let first_update = next_chunk(&mut stream).await;
        let second_update = next_chunk(&mut stream).await;
        let read_count = read_count.load(Ordering::SeqCst);

        assert_eq!(
            (initial_event, first_update, second_update, read_count),
            (
                sessions_event(31),
                sessions_event(32),
                sessions_event(33),
                3,
            ),
        );
    }

    #[tokio::test]
    async fn emits_restart_and_closes_the_event_stream() {
        let (_event_sender, event_receiver) = broadcast::channel(1);
        let revisions = RevisionMonitor::new(|| Ok(12), event_receiver, true);
        let (_enrichment_tx, enrichment_rx) = watch::channel(());
        let (restart_tx, restart_rx) = watch::channel(false);
        let stream = revision_stream(
            revisions.subscribe(),
            usage::no_updates_subscription(),
            enrichment_rx,
            restart_rx,
        );
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
        revision_stream(
            revisions,
            usage::no_updates_subscription(),
            enrichment_updates,
            restart_signal,
        )
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

    async fn next_usage_chunk(
        chunks: &mut (impl Stream<Item = Result<axum::body::Bytes, axum::Error>> + Unpin),
    ) -> String {
        let chunk = tokio::time::timeout(Duration::from_secs(3), chunks.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        String::from_utf8(chunk.to_vec()).unwrap()
    }
}
