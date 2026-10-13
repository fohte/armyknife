//! Delivers a message to a Codex thread through the persistent app-server.
//!
//! The control socket speaks WebSocket rather than newline-delimited JSON.
//! `turn/start` uses Codex's `StartOrSteer` input mode, so the same request
//! starts a turn for an idle thread or injects input into its active turn.

use std::collections::hash_map::DefaultHasher;
use std::fs::{File, OpenOptions, TryLockError};
use std::hash::{Hash, Hasher};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use serde_json::{Value, json};
use thiserror::Error;
use tungstenite::{Message, WebSocket, client};

use crate::commands::agent::types::ReasoningEffort;

const INITIALIZE_REQUEST_ID: u64 = 1;
const TURN_STATUS_READ_REQUEST_ID_START: u64 = 100;
const TURN_START_REQUEST_ID: u64 = 3;
const THREAD_ARCHIVE_REQUEST_ID: u64 = 3;
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_THREAD_STARTED_TIMEOUT: Duration = Duration::from_secs(120);
const TURN_START_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);

mod archive;
mod delivery;

pub(crate) use archive::archive_thread;

pub type Result<T> = std::result::Result<T, DeliveryError>;

#[derive(Debug, Error)]
pub enum DeliveryError {
    /// The request was rejected or the target turn ended without a matching history item.
    #[error("{0:#}")]
    NotDelivered(anyhow::Error),
    /// Delivery could not be confirmed, so queue fallback could create a duplicate.
    #[error("Codex app-server delivery is unconfirmed; not queueing to avoid a duplicate: {0:#}")]
    Unconfirmed(anyhow::Error),
}

#[derive(Debug, Error)]
enum RequestError {
    #[error("{0:#}")]
    Rejected(anyhow::Error),
    #[error("{0:#}")]
    Transport(anyhow::Error),
}

struct RpcRequest {
    id: u64,
    method: &'static str,
    payload: Value,
}

/// An initialized connection to the persistent Codex app-server.
///
/// Connect before launching a Codex pane so `thread/started` cannot be emitted
/// before this client begins listening for it.
pub struct Client {
    socket: WebSocket<UnixStream>,
    next_status_read_request_id: u64,
}

impl Client {
    /// Connects to the control socket and completes the app-server handshake.
    pub fn connect() -> anyhow::Result<Self> {
        let socket_path = control_socket_path()?;
        Self::connect_to(&socket_path)
    }

    fn connect_to(socket_path: &Path) -> anyhow::Result<Self> {
        let stream = UnixStream::connect(socket_path).with_context(|| {
            format!(
                "failed to connect to Codex app-server control socket {}",
                socket_path.display()
            )
        })?;
        stream
            .set_read_timeout(Some(RESPONSE_TIMEOUT))
            .context("failed to set Codex app-server read timeout")?;
        stream
            .set_write_timeout(Some(RESPONSE_TIMEOUT))
            .context("failed to set Codex app-server write timeout")?;

        let (mut socket, _) = client("ws://localhost/rpc", stream)
            .map_err(|error| anyhow!(error))
            .context("Codex app-server WebSocket handshake failed")?;

        send_request(&mut socket, &initialize_request()).map_err(request_error_to_anyhow)?;

        Ok(Self {
            socket,
            next_status_read_request_id: TURN_STATUS_READ_REQUEST_ID_START,
        })
    }

    /// Waits for the top-level thread created in `cwd` and returns its ID.
    pub fn wait_for_thread_started(&mut self, cwd: &Path) -> anyhow::Result<String> {
        self.wait_for_thread_started_with_timeout(cwd, DEFAULT_THREAD_STARTED_TIMEOUT)
    }

    /// Waits for the top-level thread created in `cwd` using a custom read timeout.
    pub fn wait_for_thread_started_with_timeout(
        &mut self,
        cwd: &Path,
        timeout: Duration,
    ) -> anyhow::Result<String> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .context("Codex app-server wait timeout is out of range")?;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(anyhow!(
                    "timed out waiting for Codex app-server `thread/started` notification"
                ));
            }
            self.socket
                .get_ref()
                .set_read_timeout(Some(remaining))
                .context("failed to set Codex app-server read timeout")?;
            let message = self
                .socket
                .read()
                .context("failed to read Codex app-server `thread/started` notification")?;
            if let Some(thread_id) = thread_started_from_message(message, cwd)? {
                return Ok(thread_id);
            }
        }
    }

    /// Starts the initial turn and waits for Codex to record its input.
    ///
    /// Applies `effort` to this and subsequent turns.
    /// Delivery checks can wait while an active turn has not recorded the input.
    pub fn start_turn(
        &mut self,
        thread_id: &str,
        content: &str,
        effort: Option<ReasoningEffort>,
    ) -> Result<()> {
        let client_user_message_id = uuid::Uuid::new_v4().to_string();
        delivery::send_and_confirm(
            &mut self.socket,
            thread_id,
            content,
            effort,
            &client_user_message_id,
        )
    }

    /// Reads the status of the latest turn recorded for `thread_id`.
    pub(crate) fn latest_turn_status(&mut self, thread_id: &str) -> anyhow::Result<Option<String>> {
        let request_id = self.next_status_read_request_id;
        self.next_status_read_request_id = request_id
            .checked_add(1)
            .context("Codex app-server request ID space was exhausted while reading turn status")?;
        delivery::latest_turn_status(&mut self.socket, thread_id, request_id)
    }
}

/// Serializes armyknife launches that would otherwise match the same cwd-only
/// `thread/started` notification.
pub fn acquire_launch_lock(cwd: &Path) -> anyhow::Result<File> {
    let (file, lock_path) = open_launch_lock(cwd)?;

    file.lock()
        .with_context(|| format!("failed to lock Codex launch {}", lock_path.display()))?;
    Ok(file)
}

/// Acquires the per-cwd launch lock before `timeout` elapses.
pub fn acquire_launch_lock_with_timeout(cwd: &Path, timeout: Duration) -> anyhow::Result<File> {
    let (file, lock_path) = open_launch_lock(cwd)?;
    let deadline = Instant::now()
        .checked_add(timeout)
        .context("Codex launch-lock timeout is out of range")?;

    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(TryLockError::WouldBlock) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(anyhow!(
                        "timed out waiting for another Codex launch in the same directory"
                    ));
                }
                std::thread::sleep(remaining.min(Duration::from_millis(50)));
            }
            Err(TryLockError::Error(error)) => {
                return Err(error).with_context(|| {
                    format!("failed to lock Codex launch {}", lock_path.display())
                });
            }
        }
    }
}

fn open_launch_lock(cwd: &Path) -> anyhow::Result<(File, PathBuf)> {
    let canonical_cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let mut hasher = DefaultHasher::new();
    canonical_cwd.hash(&mut hasher);
    let lock_dir = crate::shared::dirs::cache_dir()
        .context("could not determine cache directory for Codex launch lock")?
        .join("armyknife")
        .join("codex-launch-locks");
    std::fs::create_dir_all(&lock_dir).context("failed to create Codex launch lock directory")?;
    let lock_path = lock_dir.join(format!("{:016x}.lock", hasher.finish()));
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("failed to open Codex launch lock {}", lock_path.display()))?;

    Ok((file, lock_path))
}

/// Sends `content` to the Codex thread identified by `thread_id`.
pub fn send_message(thread_id: &str, content: &str) -> Result<()> {
    let mut client = Client::connect().map_err(DeliveryError::NotDelivered)?;
    client.start_turn(thread_id, content, None)
}

fn control_socket_path() -> anyhow::Result<PathBuf> {
    let codex_home = std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::shared::dirs::home_dir().map(|home| home.join(".codex")))
        .context("could not determine CODEX_HOME for the app-server control socket")?;
    Ok(codex_home
        .join("app-server-control")
        .join("app-server-control.sock"))
}

fn initialize_request() -> RpcRequest {
    RpcRequest {
        id: INITIALIZE_REQUEST_ID,
        method: "initialize",
        payload: json!({
            "id": INITIALIZE_REQUEST_ID,
            "method": "initialize",
            "params": {
                "clientInfo": {
                    "name": "armyknife",
                    "title": "armyknife",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": {
                    "experimentalApi": true,
                },
            },
        }),
    }
}

fn turn_start_request(
    thread_id: &str,
    content: &str,
    effort: Option<ReasoningEffort>,
    client_user_message_id: &str,
) -> RpcRequest {
    let mut params = json!({
        "threadId": thread_id,
        "clientUserMessageId": client_user_message_id,
        "input": [{
            "type": "text",
            "text": content,
            "textElements": [],
        }],
    });
    if let Some(effort) = effort {
        params["effort"] = json!(effort.as_str());
    }

    RpcRequest {
        id: TURN_START_REQUEST_ID,
        method: "turn/start",
        payload: json!({
            "id": TURN_START_REQUEST_ID,
            "method": "turn/start",
            "params": params,
        }),
    }
}

#[cfg(test)]
fn wait_for_thread_started<R>(mut read: R, cwd: &Path) -> anyhow::Result<String>
where
    R: FnMut() -> tungstenite::Result<Message>,
{
    loop {
        let message =
            read().context("failed to read Codex app-server `thread/started` notification")?;
        if let Some(thread_id) = thread_started_from_message(message, cwd)? {
            return Ok(thread_id);
        }
    }
}

fn thread_started_from_message(message: Message, cwd: &Path) -> anyhow::Result<Option<String>> {
    match message {
        Message::Text(text) => {
            let notification: Value = serde_json::from_str(&text)
                .context("invalid JSON in `thread/started` notification")?;
            Ok(thread_started_id(&notification, cwd))
        }
        Message::Close(frame) => Err(anyhow!(
            "Codex app-server closed while waiting for `thread/started`: {frame:?}"
        )),
        Message::Binary(_) | Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => Ok(None),
    }
}

fn thread_started_id(notification: &Value, cwd: &Path) -> Option<String> {
    if notification.get("method").and_then(Value::as_str) != Some("thread/started") {
        return None;
    }

    let thread = notification.get("params")?.get("thread")?;
    if thread
        .get("parentThreadId")
        .is_some_and(|parent| !parent.is_null())
    {
        return None;
    }
    if !paths_refer_to_same_location(Path::new(thread.get("cwd")?.as_str()?), cwd) {
        return None;
    }

    thread.get("id")?.as_str().map(str::to_string)
}

fn paths_refer_to_same_location(left: &Path, right: &Path) -> bool {
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn send_request(
    socket: &mut WebSocket<UnixStream>,
    request: &RpcRequest,
) -> std::result::Result<(), RequestError> {
    send_request_response(socket, request).map(|_| ())
}

fn send_request_response(
    socket: &mut WebSocket<UnixStream>,
    request: &RpcRequest,
) -> std::result::Result<Value, RequestError> {
    send_request_and_collect_notifications(socket, request).map(|(response, _)| response)
}

fn send_request_and_collect_notifications(
    socket: &mut WebSocket<UnixStream>,
    request: &RpcRequest,
) -> std::result::Result<(Value, Vec<Value>), RequestError> {
    socket
        .send(Message::Text(request.payload.to_string().into()))
        .with_context(|| {
            format!(
                "failed to send Codex app-server `{}` request",
                request.method
            )
        })
        .map_err(RequestError::Transport)?;
    let mut notifications = Vec::new();
    let response = wait_for_response_with_notifications(
        || socket.read(),
        request.id,
        request.method,
        |notification| notifications.push(notification),
    )?;
    Ok((response, notifications))
}

fn wait_for_response_with_notifications<R, F>(
    mut read: R,
    request_id: u64,
    method: &str,
    mut on_notification: F,
) -> std::result::Result<Value, RequestError>
where
    R: FnMut() -> tungstenite::Result<Message>,
    F: FnMut(Value),
{
    loop {
        let message = read()
            .with_context(|| format!("failed to read Codex app-server `{method}` response"))
            .map_err(RequestError::Transport)?;
        match message {
            Message::Text(text) => {
                let response: Value = serde_json::from_str(&text)
                    .with_context(|| format!("invalid JSON in `{method}` response"))
                    .map_err(RequestError::Transport)?;
                if let Some(result) = is_response_to(&response, request_id, method)? {
                    return Ok(result);
                }
                if response.get("method").and_then(Value::as_str).is_some() {
                    on_notification(response);
                }
            }
            Message::Close(frame) => {
                return Err(RequestError::Transport(anyhow!(
                    "Codex app-server closed during `{method}`: {frame:?}"
                )));
            }
            Message::Binary(_) | Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
        }
    }
}

fn is_response_to(
    response: &Value,
    request_id: u64,
    method: &str,
) -> std::result::Result<Option<Value>, RequestError> {
    if response.get("id").and_then(Value::as_u64) != Some(request_id) {
        return Ok(None);
    }
    if let Some(error) = response.get("error") {
        return Err(RequestError::Rejected(anyhow!(
            "Codex app-server `{method}` failed: {}",
            rpc_error(error)
        )));
    }
    if response.get("result").is_none() {
        return Err(RequestError::Transport(anyhow!(
            "Codex app-server `{method}` returned no result"
        )));
    }
    Ok(response.get("result").cloned())
}

fn request_error_to_anyhow(error: RequestError) -> anyhow::Error {
    match error {
        RequestError::Rejected(error) | RequestError::Transport(error) => error,
    }
}

fn rpc_error(error: &Value) -> String {
    let Some(message) = error.get("message").and_then(Value::as_str) else {
        return error.to_string();
    };
    match error.get("code").and_then(Value::as_i64) {
        Some(code) => format!("{message} (code {code})"),
        None => message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::net::Shutdown;

    use super::*;
    use rstest::rstest;

    fn connected_client_and_server() -> anyhow::Result<(Client, WebSocket<UnixStream>)> {
        let (client_stream, server_stream) = UnixStream::pair()?;
        // Bound server reads so a broken client path cannot strand the test thread.
        server_stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let server = std::thread::spawn(move || {
            tungstenite::accept(server_stream).map_err(anyhow::Error::from)
        });
        let (socket, _) = client("ws://localhost/rpc", client_stream)?;
        let server = server
            .join()
            .map_err(|_| anyhow!("Codex app-server test thread panicked"))??;

        Ok((
            Client {
                socket,
                next_status_read_request_id: TURN_STATUS_READ_REQUEST_ID_START,
            },
            server,
        ))
    }

    fn join_server<T>(
        client_socket: &WebSocket<UnixStream>,
        client_result: &std::result::Result<(), String>,
        server: std::thread::JoinHandle<anyhow::Result<T>>,
    ) -> anyhow::Result<T> {
        if client_result.is_err() {
            // Release the peer before joining so a failed client cannot leave its read blocked.
            let _ = client_socket.get_ref().shutdown(Shutdown::Both);
        }

        let server_result = server.join().map_err(|_| match client_result {
            Err(client_error) => anyhow!(
                "Codex app-server test thread panicked after client operation failed: {client_error}"
            ),
            Ok(()) => anyhow!("Codex app-server test thread panicked"),
        })?;
        server_result.map_err(|server_error| match client_result {
            Err(client_error) => {
                server_error.context(format!("client operation also failed: {client_error}"))
            }
            Ok(()) => server_error,
        })
    }

    #[test]
    fn reads_latest_turn_status_from_thread_history() {
        let (mut client, mut server) = connected_client_and_server().expect("connected sockets");
        let server_thread = std::thread::spawn(move || {
            let request = server.read().expect("thread/read request");
            let request: Value = serde_json::from_str(request.to_text().expect("text request"))
                .expect("request JSON");
            server
                .send(Message::Text(
                    json!({
                        "id": TURN_STATUS_READ_REQUEST_ID_START,
                        "result": {
                            "thread": {
                                "turns": [
                                    {"id": "turn-old", "startedAt": 10, "status": "failed"},
                                    {"id": "turn-new", "startedAt": 20, "status": "inProgress"},
                                ],
                            },
                        },
                    })
                    .to_string()
                    .into(),
                ))
                .expect("thread/read response");
            request
        });

        let status = client
            .latest_turn_status("fixture-thread-id")
            .expect("turn status");
        let request = server_thread.join().expect("server thread");
        assert_eq!(
            (request, status),
            (
                json!({
                    "id": TURN_STATUS_READ_REQUEST_ID_START,
                    "method": "thread/read",
                    "params": {
                        "threadId": "fixture-thread-id",
                        "includeTurns": true,
                    },
                }),
                Some("inProgress".to_string()),
            ),
        );
    }

    fn thread_started_message() -> Message {
        Message::Text(
            json!({
                "method": "thread/started",
                "params": {"thread": {
                    "id": "thread-a",
                    "cwd": "/workspace/project-a",
                    "parentThreadId": null,
                }},
            })
            .to_string()
            .into(),
        )
    }

    #[test]
    fn builds_initialize_request() {
        assert_eq!(
            initialize_request().payload,
            json!({
                "id": 1,
                "method": "initialize",
                "params": {
                    "clientInfo": {
                        "name": "armyknife",
                        "title": "armyknife",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                    "capabilities": {
                        "experimentalApi": true,
                    },
                },
            }),
        );
    }

    #[rstest]
    #[case::without_effort(
        None,
        json!({
            "id": TURN_START_REQUEST_ID,
            "method": "turn/start",
            "params": {
                "threadId": "thread-a",
                "clientUserMessageId": "message-a",
                "input": [{
                    "type": "text",
                    "text": "hello there",
                    "textElements": [],
                }],
            },
        })
    )]
    #[case::with_effort(
        Some(ReasoningEffort::Max),
        json!({
            "id": TURN_START_REQUEST_ID,
            "method": "turn/start",
            "params": {
                "threadId": "thread-a",
                "clientUserMessageId": "message-a",
                "input": [{
                    "type": "text",
                    "text": "hello there",
                    "textElements": [],
                }],
                "effort": "max",
            },
        })
    )]
    fn builds_turn_start_request(#[case] effort: Option<ReasoningEffort>, #[case] expected: Value) {
        assert_eq!(
            turn_start_request("thread-a", "hello there", effort, "message-a").payload,
            expected,
        );
    }

    #[rstest]
    #[case::matching_top_level_thread(
        json!({
            "method": "thread/started",
            "params": {"thread": {
                "id": "thread-a",
                "cwd": "/workspace/project-a",
                "parentThreadId": null,
                "status": {"type": "idle"},
            }},
        }),
        Some("thread-a")
    )]
    #[case::matching_thread_without_parent_field(
        json!({
            "method": "thread/started",
            "params": {"thread": {
                "id": "thread-a",
                "cwd": "/workspace/project-a",
            }},
        }),
        Some("thread-a")
    )]
    #[case::different_cwd(
        json!({
            "method": "thread/started",
            "params": {"thread": {
                "id": "thread-b",
                "cwd": "/workspace/project-b",
                "parentThreadId": null,
            }},
        }),
        None
    )]
    #[case::subagent_thread(
        json!({
            "method": "thread/started",
            "params": {"thread": {
                "id": "thread-child",
                "cwd": "/workspace/project-a",
                "parentThreadId": "thread-parent",
            }},
        }),
        None
    )]
    #[case::different_notification(
        json!({
            "method": "turn/started",
            "params": {"thread": {
                "id": "thread-a",
                "cwd": "/workspace/project-a",
                "parentThreadId": null,
            }},
        }),
        None
    )]
    #[case::missing_thread(json!({"method": "thread/started", "params": {}}), None)]
    fn selects_started_top_level_thread_in_cwd(
        #[case] notification: Value,
        #[case] expected: Option<&str>,
    ) {
        assert_eq!(
            thread_started_id(&notification, Path::new("/workspace/project-a")),
            expected.map(str::to_string),
        );
    }

    #[test]
    fn waits_through_unrelated_notifications_for_started_thread() {
        let mut messages = [
            json!({"method": "turn/started", "params": {}}),
            json!({
                "method": "thread/started",
                "params": {"thread": {
                    "id": "thread-b",
                    "cwd": "/workspace/project-b",
                    "parentThreadId": null,
                }},
            }),
            json!({
                "method": "thread/started",
                "params": {"thread": {
                    "id": "thread-a",
                    "cwd": "/workspace/project-a",
                    "parentThreadId": null,
                }},
            }),
        ]
        .into_iter()
        .map(|value| Ok(Message::Text(value.to_string().into())));

        let actual = wait_for_thread_started(
            || {
                messages
                    .next()
                    .unwrap_or_else(|| Err(tungstenite::Error::ConnectionClosed))
            },
            Path::new("/workspace/project-a"),
        );

        assert_eq!(
            actual.map_err(|error| error.to_string()),
            Ok("thread-a".to_string())
        );
    }

    #[rstest]
    #[case::closed(
        Message::Close(None),
        "Codex app-server closed while waiting for `thread/started`: None"
    )]
    #[case::invalid_json(
        Message::Text("not-json".into()),
        "invalid JSON in `thread/started` notification"
    )]
    fn reports_thread_notification_read_failure(#[case] message: Message, #[case] expected: &str) {
        let actual =
            wait_for_thread_started(|| Ok(message.clone()), Path::new("/workspace/project-a"));

        assert_eq!(
            actual.map_err(|error| error.to_string()),
            Err(expected.to_string())
        );
    }

    #[rstest]
    #[case::message_and_code(json!({"code": -32600, "message": "Not initialized"}), "Not initialized (code -32600)")]
    #[case::message_only(json!({"message": "thread not found"}), "thread not found")]
    #[case::unknown_shape(json!({"reason": "unavailable"}), r#"{"reason":"unavailable"}"#)]
    fn formats_rpc_errors(#[case] error: Value, #[case] expected: &str) {
        assert_eq!(rpc_error(&error), expected);
    }

    #[rstest]
    #[case::matching_success(
        json!({"id": 2, "result": {"turn": {"id": "turn-a"}}}),
        Ok(Some(json!({"turn": {"id": "turn-a"}})))
    )]
    #[case::notification(json!({"method": "turn/started", "params": {}}), Ok(None))]
    #[case::different_request(json!({"id": 1, "result": {}}), Ok(None))]
    #[case::server_error(
        json!({"id": 2, "error": {"code": -32600, "message": "thread not found"}}),
        Err("Codex app-server `turn/start` failed: thread not found (code -32600)")
    )]
    #[case::missing_result(
        json!({"id": 2}),
        Err("Codex app-server `turn/start` returned no result")
    )]
    fn selects_matching_rpc_response(
        #[case] response: Value,
        #[case] expected: std::result::Result<Option<Value>, &str>,
    ) {
        assert_eq!(
            is_response_to(&response, 2, "turn/start").map_err(|error| error.to_string()),
            expected.map_err(str::to_string),
        );
    }

    #[test]
    fn waits_through_notifications_for_matching_response() {
        let mut messages = [
            Ok(Message::Text(
                json!({"method": "turn/started", "params": {}})
                    .to_string()
                    .into(),
            )),
            Ok(Message::Text(
                json!({"id": 2, "result": {}}).to_string().into(),
            )),
        ]
        .into_iter();
        let mut notifications = Vec::new();
        let actual = wait_for_response_with_notifications(
            || {
                messages
                    .next()
                    .unwrap_or_else(|| Err(tungstenite::Error::ConnectionClosed))
            },
            2,
            "turn/start",
            |notification| notifications.push(notification),
        );
        assert_eq!(
            (actual.map_err(|error| error.to_string()), notifications,),
            (
                Ok(json!({})),
                vec![json!({"method": "turn/started", "params": {}})],
            ),
        );
    }

    #[test]
    fn start_turn_uses_a_fresh_response_timeout() -> anyhow::Result<()> {
        let (mut client, server_socket) = connected_client_and_server()?;
        let server = std::thread::spawn(move || -> anyhow::Result<(Value, Value, Value)> {
            let mut socket = server_socket;
            socket.send(thread_started_message())?;
            let Message::Text(request) = socket.read()? else {
                return Err(anyhow!("Codex app-server received a non-text request"));
            };
            let snapshot_request: Value = serde_json::from_str(&request)?;
            socket.send(Message::Text(
                json!({
                    "id": 2,
                    "result": {"thread": {"turns": []}},
                })
                .to_string()
                .into(),
            ))?;
            let Message::Text(request) = socket.read()? else {
                return Err(anyhow!("Codex app-server received a non-text request"));
            };
            let request: Value = serde_json::from_str(&request)?;
            let client_user_message_id = request
                .pointer("/params/clientUserMessageId")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("turn/start did not include a client user message ID"))?
                .to_owned();
            uuid::Uuid::parse_str(&client_user_message_id)
                .context("turn/start client user message ID was not a UUID")?;
            std::thread::sleep(Duration::from_millis(350));
            socket.send(Message::Text(
                json!({
                    "id": TURN_START_REQUEST_ID,
                    "result": {"turn": {"id": "turn-a", "status": "inProgress"}},
                })
                .to_string()
                .into(),
            ))?;
            let Message::Text(resume_request) = socket.read()? else {
                return Err(anyhow!("Codex app-server received a non-text request"));
            };
            let resume_request = serde_json::from_str(&resume_request)?;
            socket.send(Message::Text(
                json!({
                    "id": delivery::THREAD_RESUME_REQUEST_ID,
                    "result": {"thread": {"turns": [{
                        "id": "turn-a",
                        "status": "inProgress",
                        "items": [{
                            "type": "userMessage",
                            "clientId": client_user_message_id,
                        }],
                    }]}},
                })
                .to_string()
                .into(),
            ))?;
            Ok((snapshot_request, request, resume_request))
        });
        let thread_id = client.wait_for_thread_started_with_timeout(
            Path::new("/workspace/project-a"),
            Duration::from_millis(250),
        )?;

        let result = client
            .start_turn(&thread_id, "hello there", None)
            .map_err(|error| error.to_string());
        let (snapshot_request, request, resume_request) =
            join_server(&client.socket, &result, server)?;
        let mut normalized_request = request;
        normalized_request["params"]["clientUserMessageId"] = json!("message-a");
        let actual = (
            thread_id,
            result,
            snapshot_request,
            normalized_request,
            resume_request,
            client.socket.get_ref().read_timeout()?,
        );

        assert_eq!(
            actual,
            (
                "thread-a".to_string(),
                Ok(()),
                json!({
                    "id": 2,
                    "method": "thread/read",
                    "params": {"threadId": "thread-a", "includeTurns": true},
                }),
                json!({
                    "id": TURN_START_REQUEST_ID,
                    "method": "turn/start",
                    "params": {
                        "threadId": "thread-a",
                        "clientUserMessageId": "message-a",
                        "input": [{
                            "type": "text",
                            "text": "hello there",
                            "textElements": [],
                        }],
                    },
                }),
                json!({
                    "id": delivery::THREAD_RESUME_REQUEST_ID,
                    "method": "thread/resume",
                    "params": {"threadId": "thread-a"},
                }),
                Some(TURN_START_RESPONSE_TIMEOUT),
            ),
        );
        Ok(())
    }

    #[test]
    fn start_turn_failure_closes_the_server_connection_before_joining() -> anyhow::Result<()> {
        let (mut client, server_socket) = connected_client_and_server()?;
        server_socket
            .get_ref()
            .set_read_timeout(Some(Duration::from_secs(2)))?;
        let server = std::thread::spawn(move || -> anyhow::Result<(Value, Value, bool)> {
            let mut socket = server_socket;
            let Message::Text(snapshot_request) = socket.read()? else {
                return Err(anyhow!("Codex app-server received a non-text request"));
            };
            let snapshot_request = serde_json::from_str(&snapshot_request)?;
            socket.send(Message::Text(
                json!({
                    "id": 2,
                    "result": {"thread": {"turns": []}},
                })
                .to_string()
                .into(),
            ))?;
            let Message::Text(turn_start_request) = socket.read()? else {
                return Err(anyhow!("Codex app-server received a non-text request"));
            };
            let mut turn_start_request: Value = serde_json::from_str(&turn_start_request)?;
            turn_start_request["params"]["clientUserMessageId"] = json!("message-a");
            socket.send(Message::Text(
                json!({
                    "id": TURN_START_REQUEST_ID,
                    "error": {"code": -1, "message": "turn rejected"},
                })
                .to_string()
                .into(),
            ))?;
            let client_closed = match socket.read() {
                Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                    true
                }
                Err(tungstenite::Error::Io(error))
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset
                    ) =>
                {
                    true
                }
                Err(tungstenite::Error::Protocol(
                    tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
                )) => true,
                Ok(_) => false,
                Err(error) => return Err(error.into()),
            };
            Ok((snapshot_request, turn_start_request, client_closed))
        });

        let result = client
            .start_turn("thread-a", "hello there", None)
            .map_err(|error| error.to_string());
        let server_result =
            join_server(&client.socket, &result, server).map_err(|error| format!("{error:#}"));

        assert_eq!(
            (result, server_result),
            (
                Err("Codex app-server `turn/start` failed: turn rejected (code -1)".to_string()),
                Ok((
                    json!({
                        "id": 2,
                        "method": "thread/read",
                        "params": {"threadId": "thread-a", "includeTurns": true},
                    }),
                    json!({
                        "id": TURN_START_REQUEST_ID,
                        "method": "turn/start",
                        "params": {
                            "threadId": "thread-a",
                            "clientUserMessageId": "message-a",
                            "input": [{
                                "type": "text",
                                "text": "hello there",
                                "textElements": [],
                            }],
                        },
                    }),
                    true,
                )),
            ),
        );
        Ok(())
    }

    #[test]
    fn uses_extended_default_thread_started_timeout() -> anyhow::Result<()> {
        let (mut client, mut server) = connected_client_and_server()?;
        server.send(thread_started_message())?;
        let thread_id = client
            .wait_for_thread_started(Path::new("/workspace/project-a"))
            .map_err(|error| error.to_string());
        let timeout_exceeds_handshake = client
            .socket
            .get_ref()
            .read_timeout()?
            .is_some_and(|timeout| timeout > Duration::from_secs(30));

        assert_eq!(
            (thread_id, timeout_exceeds_handshake),
            (Ok("thread-a".to_string()), true),
        );
        Ok(())
    }

    #[rstest]
    #[case::explicit_codex_home(
        Some("/tmp/codex-home"),
        Some("/tmp/home"),
        Ok(PathBuf::from("/tmp/codex-home/app-server-control/app-server-control.sock"))
    )]
    #[case::default_codex_home(
        None,
        Some("/tmp/home"),
        Ok(PathBuf::from("/tmp/home/.codex/app-server-control/app-server-control.sock"))
    )]
    #[case::empty_codex_home(
        Some(""),
        Some("/tmp/home"),
        Ok(PathBuf::from("/tmp/home/.codex/app-server-control/app-server-control.sock"))
    )]
    #[case::missing_home(
        None,
        None,
        Err("could not determine CODEX_HOME for the app-server control socket")
    )]
    fn resolves_control_socket_path(
        #[case] codex_home: Option<&str>,
        #[case] home: Option<&str>,
        #[case] expected: std::result::Result<PathBuf, &str>,
    ) {
        let actual = temp_env::with_vars(
            [("CODEX_HOME", codex_home), ("HOME", home)],
            control_socket_path,
        );
        assert_eq!(
            actual.map_err(|error| error.to_string()),
            expected.map_err(str::to_string),
        );
    }
}
