//! Event streaming from the `tq` CLI.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};

use super::TqClient;
use crate::infra::external_tool::ExternalTool;

const EVENT_CHANNEL_CAPACITY: usize = 100;
const INITIAL_RESTART_DELAY: Duration = Duration::from_secs(1);
const MAX_RESTART_DELAY: Duration = Duration::from_secs(60);

/// A notification received from `tq events`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TqEvent {
    /// The stream connected; consumers should refresh their full snapshot.
    Connected,
    /// A tq resource changed.
    Change(TqChangeEvent),
}

/// The resource affected by a tq change event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TqChangeResource {
    Task,
    Project,
    Label,
    Queue,
    TimeBlock,
    Schedule,
    SavedView,
    DescriptionTemplate,
    RecurringTaskTemplate,
    GithubSyncRule,
    AgentSession,
    Checklist,
    ChecklistItem,
    Memo,
    Push,
    Calendar,
    Github,
    Asset,
    Integration,
    #[serde(other)]
    Unknown,
}

/// The change payload emitted by `tq events`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TqChangeEvent {
    pub resource: TqChangeResource,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub task_ids: Option<Vec<String>>,
}

/// A live subscription. Dropping it or calling [`shutdown`](Self::shutdown) stops the child.
pub struct TqEventSubscription {
    receiver: mpsc::Receiver<TqEvent>,
    _shutdown: oneshot::Sender<()>,
}

impl TqEventSubscription {
    /// Receives the next parsed event, or `None` after shutdown.
    pub async fn recv(&mut self) -> Option<TqEvent> {
        self.receiver.recv().await
    }

    /// Stops the stream and terminates its child process.
    pub fn shutdown(self) {}
}

/// Starts a subscription when the `tq` CLI is available.
pub fn subscribe_events() -> Option<TqEventSubscription> {
    TqClient::detect().map(|client| client.subscribe_events())
}

/// Parses one NDJSON line from the tq event stream.
fn parse_event_line(line: &str) -> serde_json::Result<TqEvent> {
    let value: serde_json::Value = serde_json::from_str(line)?;
    if value.get("type").and_then(serde_json::Value::as_str) == Some("connected") {
        serde_json::from_value::<ConnectedEvent>(value)?;
        Ok(TqEvent::Connected)
    } else {
        serde_json::from_value::<TqChangeEvent>(value).map(TqEvent::Change)
    }
}

#[derive(Deserialize)]
struct ConnectedEvent {
    #[serde(rename = "type")]
    _kind: String,
}

type EventReader = Box<dyn AsyncBufRead + Send + Unpin>;

struct EventProcess {
    reader: EventReader,
    child: Option<Child>,
}

impl EventProcess {
    #[cfg(test)]
    fn from_reader(reader: EventReader) -> Self {
        Self {
            reader,
            child: None,
        }
    }
}

type DelayFuture = Pin<Box<dyn Future<Output = ()> + Send>>;
type DelayFn = Box<dyn FnMut(Duration) -> DelayFuture + Send>;

pub(super) fn start_subscription() -> TqEventSubscription {
    let (sender, receiver) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    tokio::spawn(run_events(
        sender,
        shutdown_receiver,
        spawn_tq_events,
        Box::new(|delay| Box::pin(tokio::time::sleep(delay))),
    ));
    TqEventSubscription {
        receiver,
        _shutdown: shutdown_sender,
    }
}

async fn run_events<F>(
    sender: mpsc::Sender<TqEvent>,
    mut shutdown: oneshot::Receiver<()>,
    mut spawn: F,
    mut delay: DelayFn,
) where
    F: FnMut() -> io::Result<EventProcess> + Send + 'static,
{
    let mut restart_delay = INITIAL_RESTART_DELAY;

    loop {
        let mut process = match spawn() {
            Ok(process) => process,
            Err(error) => {
                tracing::warn!(%error, "failed to start tq events stream");
                if !wait_before_restart(&sender, &mut shutdown, &mut delay, restart_delay).await {
                    return;
                }
                restart_delay = next_restart_delay(restart_delay);
                continue;
            }
        };

        let mut received_change = false;
        let reached_eof = loop {
            let mut line = String::new();
            let read_result = tokio::select! {
                _ = &mut shutdown => return,
                _ = sender.closed() => return,
                result = process.reader.read_line(&mut line) => result,
            };

            match read_result {
                Ok(0) => break true,
                Ok(_) => match parse_event_line(&line) {
                    Ok(event) => {
                        if matches!(event, TqEvent::Change(_)) && !received_change {
                            restart_delay = INITIAL_RESTART_DELAY;
                            received_change = true;
                        }
                        let send_result = tokio::select! {
                            _ = &mut shutdown => return,
                            _ = sender.closed() => return,
                            result = sender.send(event) => result,
                        };
                        if send_result.is_err() {
                            return;
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, line = %line.trim_end(), "skipping invalid tq event line")
                    }
                },
                Err(error) => {
                    tracing::warn!(%error, "failed to read tq events stream");
                    break false;
                }
            }
        };

        if reached_eof && let Some(mut child) = process.child.take() {
            tokio::select! {
                _ = &mut shutdown => return,
                _ = sender.closed() => return,
                result = child.kill() => {
                    if let Err(error) = result {
                        tracing::warn!(%error, "failed to reap tq events process");
                    }
                }
            }
        }
        drop(process);

        if !wait_before_restart(&sender, &mut shutdown, &mut delay, restart_delay).await {
            return;
        }
        restart_delay = next_restart_delay(restart_delay);
    }
}

async fn wait_before_restart(
    sender: &mpsc::Sender<TqEvent>,
    shutdown: &mut oneshot::Receiver<()>,
    delay: &mut DelayFn,
    duration: Duration,
) -> bool {
    tokio::select! {
        _ = shutdown => false,
        _ = sender.closed() => false,
        _ = delay(duration) => true,
    }
}

fn next_restart_delay(current: Duration) -> Duration {
    current.saturating_mul(2).min(MAX_RESTART_DELAY)
}

fn spawn_tq_events() -> io::Result<EventProcess> {
    let mut command = Command::from(ExternalTool::Tq.command());
    command
        .arg("events")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("tq events process did not provide a stdout pipe"))?;
    Ok(EventProcess {
        reader: Box::new(BufReader::new(stdout)),
        child: Some(child),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use indoc::indoc;
    use rstest::rstest;
    use tokio::io::{AsyncWriteExt, DuplexStream};
    use tokio::sync::{Notify, mpsc};

    use super::*;

    #[rstest]
    #[case::connected(r#"{"type":"connected"}"#, TqEvent::Connected)]
    #[case::change(
        r#"{"resource":"task","id":"task-placeholder","origin":"ui","taskIds":["task-placeholder"]}"#,
        TqEvent::Change(TqChangeEvent {
            resource: TqChangeResource::Task,
            id: Some("task-placeholder".to_string()),
            origin: Some("ui".to_string()),
            task_ids: Some(vec!["task-placeholder".to_string()]),
        }),
    )]
    #[case::unknown_resource(
        r#"{"resource":"future_resource","id":null,"origin":null,"taskIds":null}"#,
        TqEvent::Change(TqChangeEvent {
            resource: TqChangeResource::Unknown,
            id: None,
            origin: None,
            task_ids: None,
        }),
    )]
    fn parses_event_lines(#[case] line: &str, #[case] expected: TqEvent) {
        assert_eq!(parse_event_line(line).unwrap(), expected);
    }

    #[test]
    fn rejects_malformed_event_lines() {
        assert!(parse_event_line("{invalid}").is_err());
    }

    #[tokio::test]
    async fn subscription_reads_each_valid_line_and_skips_invalid_lines() {
        let contents = indoc! {r#"
            {"type":"connected"}
            {invalid}
            {"resource":"project","id":"project-placeholder","origin":null,"taskIds":null}
        "#}
        .to_string();
        let mut subscription = subscription_with(
            move || Ok(stream_process(contents.as_bytes())),
            Box::new(|_| Box::pin(std::future::pending::<()>())),
        );

        let events = vec![subscription.recv().await, subscription.recv().await];
        assert_eq!(
            events,
            vec![
                Some(TqEvent::Connected),
                Some(TqEvent::Change(TqChangeEvent {
                    resource: TqChangeResource::Project,
                    id: Some("project-placeholder".to_string()),
                    origin: None,
                    task_ids: None,
                })),
            ]
        );
        subscription.shutdown();
    }

    #[tokio::test]
    async fn retries_after_spawn_failure() {
        let starts = Arc::new(AtomicUsize::new(0));
        let spawn_starts = Arc::clone(&starts);
        let spawn = move || {
            if spawn_starts.fetch_add(1, Ordering::SeqCst) < 2 {
                Err(io::Error::other("placeholder spawn failure"))
            } else {
                Ok(stream_process(b"{\"type\":\"connected\"}\n"))
            }
        };
        let (delay, mut requested_delay_receiver, notify) = gated_delay();
        let mut subscription = subscription_with(spawn, delay);

        let mut retries = vec![(
            requested_delay_receiver.recv().await,
            starts.load(Ordering::SeqCst),
        )];
        notify.notify_one();
        retries.push((
            requested_delay_receiver.recv().await,
            starts.load(Ordering::SeqCst),
        ));
        notify.notify_one();
        let event = subscription.recv().await;
        assert_eq!(
            (retries, event, starts.load(Ordering::SeqCst)),
            (
                vec![
                    (Some(Duration::from_secs(1)), 1),
                    (Some(Duration::from_secs(2)), 2),
                ],
                Some(TqEvent::Connected),
                3,
            )
        );
        subscription.shutdown();
    }

    #[tokio::test]
    async fn restarts_with_exponential_delay_and_resets_after_first_change() {
        let starts = Arc::new(AtomicUsize::new(0));
        let spawn_starts = Arc::clone(&starts);
        let spawn = move || {
            let start = spawn_starts.fetch_add(1, Ordering::SeqCst);
            let lines: &[u8] = if start == 1 {
                b"{\"type\":\"connected\"}\n"
            } else if start == 2 {
                indoc! {r#"
                    {"type":"connected"}
                    {"resource":"task","id":"task-placeholder","origin":null,"taskIds":null}
                "#}
                .as_bytes()
            } else {
                b""
            };
            Ok(stream_process(lines))
        };
        let (delay, mut requested_delay_receiver, notify) = gated_delay();
        let mut subscription = subscription_with(spawn, delay);

        let mut restarts = vec![(
            requested_delay_receiver.recv().await,
            starts.load(Ordering::SeqCst),
        )];
        let mut events = Vec::new();
        notify.notify_one();
        events.push(subscription.recv().await);
        restarts.push((
            requested_delay_receiver.recv().await,
            starts.load(Ordering::SeqCst),
        ));
        notify.notify_one();
        events.push(subscription.recv().await);
        events.push(subscription.recv().await);
        restarts.push((
            requested_delay_receiver.recv().await,
            starts.load(Ordering::SeqCst),
        ));
        notify.notify_one();
        restarts.push((
            requested_delay_receiver.recv().await,
            starts.load(Ordering::SeqCst),
        ));
        assert_eq!(
            (restarts, events),
            (
                vec![
                    (Some(Duration::from_secs(1)), 1),
                    (Some(Duration::from_secs(2)), 2),
                    (Some(Duration::from_secs(1)), 3),
                    (Some(Duration::from_secs(2)), 4),
                ],
                vec![
                    Some(TqEvent::Connected),
                    Some(TqEvent::Connected),
                    Some(TqEvent::Change(TqChangeEvent {
                        resource: TqChangeResource::Task,
                        id: Some("task-placeholder".to_string()),
                        origin: None,
                        task_ids: None,
                    })),
                ],
            )
        );
        subscription.shutdown();
    }

    fn subscription_with<F>(spawn: F, delay: DelayFn) -> TqEventSubscription
    where
        F: FnMut() -> io::Result<EventProcess> + Send + 'static,
    {
        let (sender, receiver) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
        let (shutdown_sender, shutdown_receiver) = oneshot::channel();
        tokio::spawn(run_events(sender, shutdown_receiver, spawn, delay));
        TqEventSubscription {
            receiver,
            _shutdown: shutdown_sender,
        }
    }

    fn gated_delay() -> (DelayFn, mpsc::UnboundedReceiver<Duration>, Arc<Notify>) {
        let (requested_delay_sender, requested_delay_receiver) = mpsc::unbounded_channel();
        let notify = Arc::new(Notify::new());
        let delay_notify = Arc::clone(&notify);
        let delay: DelayFn = Box::new(move |duration| {
            let _ = requested_delay_sender.send(duration);
            let delay_notify = Arc::clone(&delay_notify);
            Box::pin(async move { delay_notify.notified().await })
        });
        (delay, requested_delay_receiver, notify)
    }

    fn stream_process(contents: &[u8]) -> EventProcess {
        let (writer, reader) = tokio::io::duplex(1024);
        tokio::spawn(write_stream(writer, contents.to_vec()));
        EventProcess::from_reader(Box::new(BufReader::new(reader)))
    }

    async fn write_stream(mut writer: DuplexStream, contents: Vec<u8>) {
        if let Err(error) = writer.write_all(&contents).await {
            panic!("failed to write test stream: {error}");
        }
    }
}
