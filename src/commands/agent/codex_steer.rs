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
const TURN_START_REQUEST_ID: u64 = 2;
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);

mod archive;

pub(crate) use archive::archive_thread;

pub type Result<T> = std::result::Result<T, DeliveryError>;

#[derive(Debug, Error)]
pub enum DeliveryError {
    /// The app-server did not accept `turn/start`, so queue fallback is safe.
    #[error("{0:#}")]
    NotDelivered(anyhow::Error),
    /// `turn/start` was written but no authoritative response arrived.
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

        Ok(Self { socket })
    }

    /// Waits for the top-level thread created in `cwd` and returns its ID.
    pub fn wait_for_thread_started(&mut self, cwd: &Path) -> anyhow::Result<String> {
        self.wait_for_thread_started_with_timeout(cwd, RESPONSE_TIMEOUT)
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

    /// Starts the initial turn, applying `effort` to this and subsequent turns.
    pub fn start_turn(
        &mut self,
        thread_id: &str,
        content: &str,
        effort: Option<ReasoningEffort>,
    ) -> Result<()> {
        send_request(
            &mut self.socket,
            &turn_start_request(thread_id, content, effort),
        )
        .map_err(|error| match error {
            RequestError::Rejected(error) => DeliveryError::NotDelivered(error),
            RequestError::Transport(error) => DeliveryError::Unconfirmed(error),
        })
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
) -> RpcRequest {
    let mut params = json!({
        "threadId": thread_id,
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
    send_request_with_result(socket, request).map(|_| ())
}

fn send_request_with_result(
    socket: &mut WebSocket<UnixStream>,
    request: &RpcRequest,
) -> std::result::Result<Value, RequestError> {
    socket
        .send(Message::Text(request.payload.to_string().into()))
        .with_context(|| {
            format!(
                "failed to send Codex app-server `{}` request",
                request.method
            )
        })
        .map_err(RequestError::Transport)?;
    wait_for_response(|| socket.read(), request.id, request.method)
}

fn wait_for_response<R>(
    mut read: R,
    request_id: u64,
    method: &str,
) -> std::result::Result<Value, RequestError>
where
    R: FnMut() -> tungstenite::Result<Message>,
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
    use super::*;
    use rstest::rstest;

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
            "id": 2,
            "method": "turn/start",
            "params": {
                "threadId": "thread-a",
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
            "id": 2,
            "method": "turn/start",
            "params": {
                "threadId": "thread-a",
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
            turn_start_request("thread-a", "hello there", effort).payload,
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
        let actual = wait_for_response(
            || {
                messages
                    .next()
                    .unwrap_or_else(|| Err(tungstenite::Error::ConnectionClosed))
            },
            2,
            "turn/start",
        );
        assert_eq!(actual.map_err(|error| error.to_string()), Ok(json!({})));
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
