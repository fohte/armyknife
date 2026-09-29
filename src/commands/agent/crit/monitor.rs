//! Detached cleanup worker for crit review daemons.

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use clap::Args;

use super::super::store;
use super::super::types::Session;
use super::pane;
use super::url::parse_port;
use crate::infra::external_tool::ExternalTool;
use crate::infra::process;
use crate::infra::tmux;
use crate::shared::log::short_run_id;

const MONITOR_POLL_INTERVAL: Duration = Duration::from_secs(1);
const STARTING_MARKER_TTL: Duration = Duration::from_secs(15);

#[derive(Args, Clone, PartialEq, Eq)]
pub struct MonitorArgs {
    /// Crit daemon port.
    #[arg(long)]
    port: u16,

    /// Crit daemon process ID.
    #[arg(long)]
    pid: u32,

    /// Agent session ID that owns the review, if any.
    #[arg(long)]
    session: Option<String>,

    /// Pane associated when the review was requested.
    #[arg(long)]
    pane: Option<String>,

    /// Reservation marker claimed by the process that launched this worker.
    #[arg(long)]
    reservation: String,
}

pub(super) fn ensure_started(port: u16, session: Option<&Session>) -> Result<()> {
    let marker = marker_path(port)?;
    let Some(reservation) = claim_marker(&marker)? else {
        return Ok(());
    };
    let pane_id = session
        .and_then(|session| session.tmux_info.as_ref().map(|info| info.pane_id.as_str()))
        .unwrap_or_default();
    let session_id = session.map(|session| session.session_id.as_str());

    let result = (|| {
        let pid = crit_pid_for_port(port)?
            .with_context(|| format!("crit status did not report a daemon on port {port}"))?;
        let executable =
            std::env::current_exe().context("failed to resolve the armyknife executable")?;
        let mut args = vec![
            "agent".to_string(),
            "crit".to_string(),
            "monitor".to_string(),
            format!("--port={port}"),
            format!("--pid={pid}"),
            format!("--reservation={reservation}"),
        ];
        if let Some(session_id) = session_id {
            args.push(format!("--session={session_id}"));
        }
        if !pane_id.is_empty() {
            args.push(format!("--pane={pane_id}"));
        }

        let worker_pid =
            process::spawn_detached_with_pid(&executable, args, Some(Path::new("/")), &[])
                .context("failed to spawn crit lifecycle monitor")?;
        tracing::info!(
            event = "agent.crit.monitor.started",
            port,
            worker_pid,
            session = session_id.unwrap_or("none"),
        );
        Ok(())
    })();

    if result.is_err() {
        remove_marker_if_equals(&marker, &reservation);
    }
    result
}

pub(super) fn run(args: &MonitorArgs) -> Result<()> {
    let span = tracing::info_span!(
        "agent.crit.monitor",
        run_id = %short_run_id(),
        session = args.session.as_deref().unwrap_or("none"),
        port = args.port,
    );
    let _guard = span.enter();
    let marker = marker_path(args.port)?;
    write_marker_owner(&marker, &args.reservation)?;
    tracing::info!(event = "agent.crit.monitor.start", daemon_pid = args.pid);
    let mut watched_pid = args.pid;
    loop {
        while process::is_process_alive(watched_pid) {
            thread::sleep(MONITOR_POLL_INTERVAL);
        }

        match crit_pid_for_port(args.port) {
            Ok(Some(current_pid))
                if current_pid != watched_pid && process::is_process_alive(current_pid) =>
            {
                tracing::info!(
                    event = "agent.crit.monitor.replacement_found",
                    previous_pid = watched_pid,
                    current_pid,
                );
                watched_pid = current_pid;
            }
            Ok(_) => break,
            Err(error) => {
                tracing::warn!(
                    event = "agent.crit.monitor.replacement_check_failed",
                    error = %error,
                );
                break;
            }
        }
    }

    kill_shpool_session(args.port);
    close_crit_panes(args.port);
    cleanup_session_links(args.port, args.session.as_deref(), args.pane.as_deref());
    remove_marker_if_owned(&marker);
    tracing::info!(event = "agent.crit.monitor.exit", daemon_pid = watched_pid);
    Ok(())
}

fn crit_pid_for_port(port: u16) -> Result<Option<u32>> {
    let mut command = ExternalTool::Crit.command();
    command.args(["status", "--json"]);
    let output = process::run_with_timeout(command, Duration::from_secs(5))
        .context("failed to read crit daemon status")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("crit status --json failed: {stderr}");
    }

    let status: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("failed to parse crit status JSON")?;
    Ok(pid_for_port(&status, port))
}

fn pid_for_port(status: &serde_json::Value, port: u16) -> Option<u32> {
    status
        .get("sessions")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .find(|session| {
            session.get("port").and_then(serde_json::Value::as_u64) == Some(port.into())
        })
        .and_then(|session| session.get("pid").and_then(serde_json::Value::as_u64))
        .and_then(|pid| u32::try_from(pid).ok())
}

fn marker_path(port: u16) -> Result<PathBuf> {
    Ok(store::crit_dir()?.join(format!("{port}.monitor")))
}

fn claim_marker(path: &Path) -> Result<Option<String>> {
    let _lock = lock_marker(path)?;
    if marker_is_active(path)? {
        return Ok(None);
    }

    let reservation = format!("starting:{}:{}", std::process::id(), unix_time_secs());
    fs::write(path, &reservation)?;
    Ok(Some(reservation))
}

fn marker_is_active(path: &Path) -> Result<bool> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    Ok(marker_is_active_at(
        &contents,
        unix_time_secs(),
        process::is_process_alive,
    ))
}

fn marker_is_active_at(contents: &str, now: u64, is_process_alive: impl Fn(u32) -> bool) -> bool {
    if let Ok(pid) = contents.trim().parse::<u32>() {
        return is_process_alive(pid);
    }
    let Some((pid, started_at)) = contents
        .trim()
        .strip_prefix("starting:")
        .and_then(|value| value.split_once(':'))
    else {
        return false;
    };
    let (Ok(pid), Ok(started_at)) = (pid.parse::<u32>(), started_at.parse::<u64>()) else {
        return false;
    };
    is_process_alive(pid) && now.saturating_sub(started_at) < STARTING_MARKER_TTL.as_secs()
}

fn lock_marker(path: &Path) -> Result<File> {
    let parent = path.parent().context("crit monitor marker has no parent")?;
    fs::create_dir_all(parent)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(parent.join(".monitor.lock"))?;
    file.lock()?;
    Ok(file)
}

fn remove_marker_if_equals(path: &Path, expected: &str) {
    let Ok(_lock) = lock_marker(path) else {
        return;
    };
    if fs::read_to_string(path)
        .ok()
        .is_some_and(|contents| contents.trim() == expected)
    {
        let _ = fs::remove_file(path);
    }
}

fn write_marker_owner(path: &Path, reservation: &str) -> Result<()> {
    let _lock = lock_marker(path)?;
    if fs::read_to_string(path)
        .ok()
        .is_none_or(|contents| contents.trim() != reservation)
    {
        bail!("crit monitor reservation expired before the worker started");
    }
    fs::write(path, std::process::id().to_string())
        .with_context(|| format!("failed to update crit monitor marker {}", path.display()))
}

fn remove_marker_if_owned(path: &Path) {
    let Ok(_lock) = lock_marker(path) else {
        return;
    };
    let marker_pid = fs::read_to_string(path)
        .ok()
        .and_then(|contents| contents.trim().parse::<u32>().ok());
    if marker_pid == Some(std::process::id()) {
        let _ = fs::remove_file(path);
    }
}

fn kill_shpool_session(port: u16) {
    let session = format!("crit-{port}");
    let result = ExternalTool::Shpool
        .command()
        .args(["kill", session.as_str()])
        .status();
    match result {
        Ok(status) if status.success() => {}
        Ok(status) => tracing::debug!(
            event = "agent.crit.cleanup.shpool_already_stopped",
            port,
            %status,
        ),
        Err(error) => tracing::warn!(
            event = "agent.crit.cleanup.shpool_kill_failed",
            port,
            error = %error,
        ),
    }
}

fn cleanup_session_links(
    port: u16,
    fallback_session_id: Option<&str>,
    fallback_pane_id: Option<&str>,
) {
    let mut fallback_was_processed = fallback_session_id.is_none();
    match store::list_all_sessions() {
        Ok(sessions) => {
            for session in sessions.into_iter().filter(|session| {
                session
                    .crit_urls
                    .iter()
                    .any(|url| parse_port(url).ok() == Some(port))
            }) {
                fallback_was_processed |= Some(session.session_id.as_str()) == fallback_session_id;
                let pane_id = session.tmux_info.as_ref().map(|info| info.pane_id.as_str());
                remove_session_link(port, &session.session_id, pane_id);
            }
        }
        Err(error) => {
            tracing::warn!(
                event = "agent.crit.cleanup.session_list_failed",
                port,
                error = %error,
            );
        }
    }
    if !fallback_was_processed && let Some(fallback_session_id) = fallback_session_id {
        remove_session_link(port, fallback_session_id, fallback_pane_id);
    }
}

fn close_crit_panes(port: u16) {
    let panes = match tmux::find_crit_panes_for_port(port) {
        Ok(panes) => panes,
        Err(error) => {
            tracing::warn!(event = "agent.crit.cleanup.pane_list_failed", port, error = %error);
            return;
        }
    };
    for pane_id in panes {
        if let Err(error) = tmux::close_crit_pane(&pane_id) {
            tracing::warn!(
                event = "agent.crit.cleanup.pane_close_failed",
                port,
                pane = %pane_id,
                error = %error,
            );
        }
    }
}

fn remove_session_link(port: u16, session_id: &str, fallback_pane_id: Option<&str>) {
    let sessions_dir = match store::sessions_dir() {
        Ok(dir) => dir,
        Err(error) => {
            tracing::warn!(
                event = "agent.crit.cleanup.session_dir_failed",
                session = %session_id,
                port,
                error = %error,
            );
            restore_pane_link(port, fallback_pane_id, None, session_id);
            return;
        }
    };
    let updated = match drop_port_from_session_in(&sessions_dir, port, session_id) {
        Ok(updated) => updated,
        Err(error) => {
            tracing::warn!(
                event = "agent.crit.cleanup.session_update_failed",
                session = %session_id,
                port,
                error = %error,
            );
            None
        }
    };
    let pane_id = updated
        .as_ref()
        .and_then(|session| session.tmux_info.as_ref())
        .map(|info| info.pane_id.as_str())
        .or(fallback_pane_id);
    let latest_url = updated
        .as_ref()
        .and_then(|session| session.crit_urls.last().map(String::as_str));
    restore_pane_link(port, pane_id, latest_url, session_id);
}

fn drop_port_from_session_in(
    sessions_dir: &Path,
    port: u16,
    session_id: &str,
) -> Result<Option<Session>> {
    let lock = store::lock_session_for_update(sessions_dir, session_id)?;
    let Some(mut session) = lock.load()? else {
        return Ok(None);
    };

    let previous_len = session.crit_urls.len();
    session
        .crit_urls
        .retain(|url| parse_port(url).ok() != Some(port));
    if session.crit_urls.len() != previous_len {
        lock.save(&session)?;
    }
    Ok(Some(session))
}

fn restore_pane_link(port: u16, pane_id: Option<&str>, latest_url: Option<&str>, session: &str) {
    let Some(pane_id) = pane_id else {
        return;
    };
    if let Err(error) = pane::restore_crit_url(pane_id, port, latest_url) {
        tracing::warn!(
            event = "agent.crit.cleanup.pane_sync_failed",
            session,
            pane = %pane_id,
            port,
            error = %error,
        );
    }
}

fn unix_time_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}
