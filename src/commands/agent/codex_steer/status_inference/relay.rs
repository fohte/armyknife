use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use reqwest::redirect::Policy;
use tokio::runtime::Handle;

mod http;
mod websocket;

const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(20);
const CONNECTION_PROTOCOL_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_ACTIVE_CONNECTIONS: usize = 4;
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Clone)]
struct RelayState {
    path: String,
    authority: String,
    parent_session_id: String,
    client: reqwest::Client,
    runtime: Handle,
    cancelled: Arc<AtomicBool>,
    active_connections: Arc<AtomicUsize>,
}

pub(super) struct Relay {
    port: u16,
    path: String,
    authority: String,
    cancelled: Arc<AtomicBool>,
    listener_thread: Option<JoinHandle<()>>,
}

impl Relay {
    pub(super) fn start(parent_session_id: &str) -> Result<Self> {
        let runtime =
            Handle::try_current().context("Codex status relay requires the app runtime")?;
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .timeout(UPSTREAM_TIMEOUT)
            .build()
            .context("create official Codex API client")?;
        let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .context("bind temporary Codex status relay")?;
        listener
            .set_nonblocking(true)
            .context("set temporary Codex status relay nonblocking")?;
        let port = listener
            .local_addr()
            .context("read temporary Codex status relay address")?
            .port();
        let authority = child_authority(port);
        let path = format!("/{}/backend-api/codex/responses", uuid::Uuid::new_v4());
        let cancelled = Arc::new(AtomicBool::new(false));
        let active_connections = Arc::new(AtomicUsize::new(0));
        let state = RelayState {
            path: path.clone(),
            authority: authority.clone(),
            parent_session_id: parent_session_id.to_string(),
            client,
            runtime,
            cancelled: cancelled.clone(),
            active_connections,
        };
        let listener_thread = thread::Builder::new()
            .name("armyknife-codex-status-relay".to_string())
            .spawn(move || accept_loop(listener, state))
            .context("start temporary Codex status relay")?;
        Ok(Self {
            port,
            path,
            authority,
            cancelled,
            listener_thread: Some(listener_thread),
        })
    }

    pub(super) fn port(&self) -> u16 {
        self.port
    }

    pub(super) fn base_url(&self) -> String {
        format!(
            "http://{}{}",
            self.authority,
            self.path.trim_end_matches("/responses")
        )
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        if let Some(listener_thread) = self.listener_thread.take() {
            let _ = listener_thread.join();
        }
    }
}

fn accept_loop(listener: TcpListener, state: RelayState) {
    let mut connections = Vec::new();
    while !state.cancelled.load(Ordering::Relaxed) {
        reap_finished_connections(&mut connections);
        match listener.accept() {
            Ok((mut stream, address)) => {
                if !address.ip().is_loopback() {
                    continue;
                }
                let Some(permit) = try_acquire_connection(&state.active_connections) else {
                    http::write_rejection(&mut stream, 503, "relay busy");
                    continue;
                };
                let connection_state = state.clone();
                if let Ok(connection) = thread::Builder::new()
                    .name("armyknife-codex-status-relay-connection".to_string())
                    .spawn(move || {
                        let _permit = permit;
                        handle_connection(stream, address, connection_state);
                    })
                {
                    connections.push(connection);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(ACCEPT_POLL_INTERVAL);
            }
            Err(_) => thread::sleep(ACCEPT_POLL_INTERVAL),
        }
    }
    for connection in connections {
        let _ = connection.join();
    }
}

struct ConnectionPermit(Arc<AtomicUsize>);

impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

fn try_acquire_connection(active_connections: &Arc<AtomicUsize>) -> Option<ConnectionPermit> {
    active_connections
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |active| {
            (active < MAX_ACTIVE_CONNECTIONS).then_some(active + 1)
        })
        .ok()
        .map(|_| ConnectionPermit(active_connections.clone()))
}

fn reap_finished_connections(connections: &mut Vec<JoinHandle<()>>) {
    let mut active = Vec::with_capacity(connections.len());
    for connection in connections.drain(..) {
        if connection.is_finished() {
            let _ = connection.join();
        } else {
            active.push(connection);
        }
    }
    *connections = active;
}

fn child_authority(port: u16) -> String {
    #[cfg(target_os = "linux")]
    {
        let _ = port;
        "localhost:3128".to_string()
    }
    #[cfg(not(target_os = "linux"))]
    {
        format!("localhost:{port}")
    }
}

fn handle_connection(mut stream: TcpStream, address: SocketAddr, state: RelayState) {
    if !address.ip().is_loopback() || state.cancelled.load(Ordering::Relaxed) {
        return;
    }
    if stream.set_read_timeout(Some(ACCEPT_POLL_INTERVAL)).is_err()
        || stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .is_err()
    {
        return;
    }

    match connection_protocol(&stream, &state.cancelled) {
        Some(ConnectionProtocol::Http) => {
            let _ = http::handle(&mut stream, &state);
        }
        Some(ConnectionProtocol::WebSocket) => {
            let _ = websocket::handle(stream, &state);
        }
        None => http::write_rejection(&mut stream, 400, "unsupported request"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionProtocol {
    Http,
    WebSocket,
}

fn connection_protocol(stream: &TcpStream, cancelled: &AtomicBool) -> Option<ConnectionProtocol> {
    let mut prefix = [0u8; 4];
    let deadline = Instant::now() + CONNECTION_PROTOCOL_TIMEOUT;
    loop {
        if cancelled.load(Ordering::Relaxed) || Instant::now() >= deadline {
            return None;
        }
        match stream.peek(&mut prefix) {
            Ok(0) => return None,
            Ok(1..=3) => thread::sleep(ACCEPT_POLL_INTERVAL),
            Ok(_) => return protocol_from_prefix(&prefix),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                thread::sleep(ACCEPT_POLL_INTERVAL);
            }
            Err(_) => return None,
        }
    }
}

fn protocol_from_prefix(prefix: &[u8]) -> Option<ConnectionProtocol> {
    match prefix {
        b"POST" => Some(ConnectionProtocol::Http),
        b"GET " => Some(ConnectionProtocol::WebSocket),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    use rstest::rstest;

    use super::{
        ConnectionProtocol, MAX_ACTIVE_CONNECTIONS, child_authority, protocol_from_prefix,
        try_acquire_connection,
    };

    #[test]
    fn relay_connection_capacity_is_bounded_and_released() {
        let active_connections = Arc::new(AtomicUsize::new(0));
        let mut permits = (0..MAX_ACTIVE_CONNECTIONS)
            .map(|_| try_acquire_connection(&active_connections))
            .collect::<Option<Vec<_>>>()
            .expect("the configured relay capacity should be available");
        let overflow = try_acquire_connection(&active_connections);
        assert_eq!(
            (
                permits.len(),
                overflow.is_some(),
                active_connections.load(std::sync::atomic::Ordering::Relaxed),
            ),
            (MAX_ACTIVE_CONNECTIONS, false, MAX_ACTIVE_CONNECTIONS),
        );

        drop(permits.pop());
        let replacement = try_acquire_connection(&active_connections);
        assert_eq!(
            (
                replacement.is_some(),
                active_connections.load(std::sync::atomic::Ordering::Relaxed),
            ),
            (true, MAX_ACTIVE_CONNECTIONS),
        );
    }

    #[test]
    fn child_authority_uses_the_srt_linux_bridge_when_available() {
        let expected = if cfg!(target_os = "linux") {
            "localhost:3128".to_string()
        } else {
            "localhost:43817".to_string()
        };
        assert_eq!(child_authority(43817), expected);
    }

    #[rstest]
    #[case::post(b"POST", Some(ConnectionProtocol::Http))]
    #[case::websocket_get(b"GET ", Some(ConnectionProtocol::WebSocket))]
    #[case::connect(b"CONN", None)]
    fn protocol_prefix_selects_only_http_or_websocket(
        #[case] request_prefix: &'static [u8],
        #[case] expected: Option<ConnectionProtocol>,
    ) {
        assert_eq!(protocol_from_prefix(request_prefix), expected);
    }
}
