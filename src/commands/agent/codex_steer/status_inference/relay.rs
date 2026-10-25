use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::redirect::Policy;
use tokio::runtime::Handle;

mod http;
mod websocket;

const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(20);
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Clone)]
struct RelayState {
    path: String,
    authority: String,
    parent_session_id: String,
    client: reqwest::Client,
    runtime: Handle,
    cancelled: Arc<AtomicBool>,
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
        #[cfg(target_os = "linux")]
        let authority = "localhost:3128".to_string();
        #[cfg(not(target_os = "linux"))]
        let authority = format!("localhost:{port}");
        let path = format!("/{}/backend-api/codex/responses", uuid::Uuid::new_v4());
        let cancelled = Arc::new(AtomicBool::new(false));
        let state = RelayState {
            path: path.clone(),
            authority: authority.clone(),
            parent_session_id: parent_session_id.to_string(),
            client,
            runtime,
            cancelled: cancelled.clone(),
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
        match listener.accept() {
            Ok((stream, address)) => {
                let connection_state = state.clone();
                if let Ok(connection) = thread::Builder::new()
                    .name("armyknife-codex-status-relay-connection".to_string())
                    .spawn(move || handle_connection(stream, address, connection_state))
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
    loop {
        if cancelled.load(Ordering::Relaxed) {
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
    use rstest::rstest;

    use super::{ConnectionProtocol, protocol_from_prefix};

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
