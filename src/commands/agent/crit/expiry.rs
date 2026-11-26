use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};

use super::super::store;
use super::super::types::Session;
use super::monitor::crit_pid_for_port;
use super::url::{local_crit_ip, parse_port};

pub(super) fn stop_expired_session_daemons(session: &Session, sessions_dir: &Path) -> Result<()> {
    stop_expired_session_daemons_with(
        session,
        sessions_dir,
        store::all_session_rows_from,
        crit_pid_for_port,
        request_crit_shutdown,
    )
}

fn stop_expired_session_daemons_with(
    session: &Session,
    sessions_dir: &Path,
    load_remaining_sessions: impl FnOnce(&Path) -> Result<Vec<Session>>,
    resolve_pid: impl FnMut(u16) -> Result<Option<u32>>,
    request_shutdown: impl FnMut(&str, u32) -> Result<()>,
) -> Result<()> {
    let candidate_ports = session
        .crit_urls
        .iter()
        .filter_map(|url| parse_port(url).ok())
        .collect::<BTreeSet<_>>();
    if candidate_ports.is_empty() {
        return Ok(());
    }

    let _port_locks = candidate_ports
        .iter()
        .map(|port| store::lock_crit_port_from(sessions_dir, *port))
        .collect::<Result<Vec<_>>>()?;
    let remaining_sessions = load_remaining_sessions(sessions_dir)?;
    let ports = unshared_crit_ports(session, &remaining_sessions);
    if ports.is_empty() {
        return Ok(());
    }

    for failure in stop_daemons_for_ports_with(session, &ports, resolve_pid, request_shutdown) {
        tracing::warn!(
            event = "agent.crit.expiry.shutdown_failed",
            session_id = %session.session_id,
            port = failure.port,
            error = %failure.error,
        );
    }
    Ok(())
}

fn unshared_crit_ports(session: &Session, remaining_sessions: &[Session]) -> BTreeSet<u16> {
    let linked_elsewhere = remaining_sessions
        .iter()
        .flat_map(|session| session.crit_urls.iter())
        .filter_map(|url| parse_port(url).ok())
        .collect::<BTreeSet<_>>();

    session
        .crit_urls
        .iter()
        .filter_map(|url| parse_port(url).ok())
        .filter(|port| !linked_elsewhere.contains(port))
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
struct StopFailure {
    port: u16,
    error: String,
}

fn stop_daemons_for_ports_with(
    session: &Session,
    ports: &BTreeSet<u16>,
    mut resolve_pid: impl FnMut(u16) -> Result<Option<u32>>,
    mut request_shutdown: impl FnMut(&str, u32) -> Result<()>,
) -> Vec<StopFailure> {
    let mut failures = Vec::new();
    for port in ports {
        let Some(url) = session
            .crit_urls
            .iter()
            .find(|url| parse_port(url).is_ok_and(|url_port| url_port == *port))
        else {
            failures.push(StopFailure {
                port: *port,
                error: "crit URL for port is missing".to_string(),
            });
            continue;
        };

        match resolve_pid(*port) {
            Ok(Some(pid)) => {
                if let Err(error) = request_shutdown(url, pid) {
                    failures.push(StopFailure {
                        port: *port,
                        error: error.to_string(),
                    });
                }
            }
            Ok(None) => failures.push(StopFailure {
                port: *port,
                error: "no crit daemon is listening on the port".to_string(),
            }),
            Err(error) => failures.push(StopFailure {
                port: *port,
                error: error.to_string(),
            }),
        }
    }

    failures
}

const SHUTDOWN_REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

fn request_crit_shutdown(url: &str, pid: u32) -> Result<()> {
    let (address, host_header) = shutdown_request_address(url)?;
    let mut stream = TcpStream::connect_timeout(&address, SHUTDOWN_REQUEST_TIMEOUT)
        .with_context(|| format!("connecting to crit daemon at {address}"))?;
    stream.set_read_timeout(Some(SHUTDOWN_REQUEST_TIMEOUT))?;
    stream.set_write_timeout(Some(SHUTDOWN_REQUEST_TIMEOUT))?;
    let request = build_shutdown_request(pid, &host_header);
    stream.write_all(request.as_bytes())?;

    let mut status_line = String::new();
    BufReader::new(stream)
        .read_line(&mut status_line)
        .context("reading crit shutdown response")?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .context("crit shutdown returned an invalid HTTP status line")?
        .parse::<u16>()
        .context("crit shutdown returned an invalid HTTP status code")?;
    if status == 409 {
        bail!("crit daemon rejected shutdown because its PID changed");
    }
    ensure!(
        (200..300).contains(&status),
        "crit daemon returned HTTP {status} for shutdown"
    );
    Ok(())
}

fn build_shutdown_request(pid: u32, host_header: &str) -> String {
    indoc::formatdoc! {"
        POST /api/shutdown?pid={pid} HTTP/1.1\r
        Host: {host_header}\r
        Content-Length: 0\r
        Connection: close\r
        \r
    "}
}

fn shutdown_request_address(url: &str) -> Result<(SocketAddr, String)> {
    let parsed = reqwest::Url::parse(url).context("invalid crit review URL")?;
    ensure!(
        parsed.scheme() == "http",
        "crit shutdown requires an HTTP URL"
    );
    let host = parsed.host_str().context("crit review URL has no host")?;
    let ip = local_crit_ip(host).context("crit review URL is not a loopback address")?;
    let port = parse_port(url)?;
    let host_header = match ip {
        IpAddr::V4(_) => format!("{host}:{port}"),
        IpAddr::V6(_) => {
            let host = host
                .strip_prefix('[')
                .and_then(|host| host.strip_suffix(']'))
                .unwrap_or(host);
            format!("[{host}]:{port}")
        }
    };
    Ok((SocketAddr::new(ip, port), host_header))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::commands::agent::types::{Engine, SessionStatus};

    fn make_session(session_id: &str, crit_urls: &[&str]) -> Session {
        Session {
            launch_id: None,
            archived_at: None,
            pr_context: None,
            created_pr_urls: Vec::new(),
            session_id: session_id.to_string(),
            crit_urls: crit_urls.iter().map(|url| (*url).to_string()).collect(),
            crit_not_waiting_urls: Default::default(),
            web_uis: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: Path::new("/tmp/example-worktree").to_path_buf(),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Ended,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            last_prompted_at: None,
            last_message: None,
            current_tool: None,
            current_tool_name: None,
            label: None,
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            child_launch: Default::default(),
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: Default::default(),
            pending_agent_task_ids: Default::default(),
            pending_permission_agent_ids: Default::default(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            agent_status: None,
            agent_status_clear_pending: false,
            engine: Engine::Claude,
            model: None,
            effort: None,
        }
    }

    #[test]
    fn requests_shutdown_only_for_unshared_crit_daemons() {
        let temp_dir = tempfile::TempDir::new().expect("temporary directory should be created");
        let expired = make_session(
            "expired-session",
            &[
                "http://review.example.test:43120/review",
                "http://review.example.test:43121/review",
                "http://review.example.test:43120/another-path",
            ],
        );
        let mut remaining = make_session(
            "remaining-session",
            &["http://review.example.test:43120/review"],
        );
        remaining.archived_at = Some(chrono::Utc::now());
        store::save_session_to(temp_dir.path(), &remaining)
            .expect("remaining session should be saved");
        let resolved_ports = RefCell::new(Vec::new());
        let shutdowns = RefCell::new(Vec::new());

        stop_expired_session_daemons_with(
            &expired,
            temp_dir.path(),
            store::all_session_rows_from,
            |port| {
                resolved_ports.borrow_mut().push(port);
                Ok(Some(9121))
            },
            |url, pid| {
                shutdowns.borrow_mut().push((url.to_string(), pid));
                Ok(())
            },
        )
        .expect("unshared daemon should be processed");

        assert_eq!(
            (resolved_ports.into_inner(), shutdowns.into_inner()),
            (
                vec![43121],
                vec![("http://review.example.test:43121/review".to_string(), 9121)],
            ),
        );
    }

    #[test]
    fn warns_when_lsof_finds_no_crit_listener_for_the_port() {
        let session = make_session(
            "expired-session",
            &["http://review.example.test:43122/review"],
        );
        let ports = BTreeSet::from([43122]);
        let shutdowns = RefCell::new(Vec::new());

        let failures = stop_daemons_for_ports_with(
            &session,
            &ports,
            |_| Ok(None),
            |url, pid| {
                shutdowns.borrow_mut().push((url.to_string(), pid));
                Ok(())
            },
        );

        assert_eq!(
            (failures, shutdowns.into_inner()),
            (
                vec![StopFailure {
                    port: 43122,
                    error: "no crit daemon is listening on the port".to_string(),
                }],
                vec![],
            ),
        );
    }

    #[test]
    fn builds_a_pid_guarded_shutdown_request_without_browser_headers() {
        assert_eq!(
            build_shutdown_request(1234, "127.0.0.1:43123"),
            indoc::indoc! {"
                POST /api/shutdown?pid=1234 HTTP/1.1\r
                Host: 127.0.0.1:43123\r
                Content-Length: 0\r
                Connection: close\r
                \r
            "},
        );
    }
}
