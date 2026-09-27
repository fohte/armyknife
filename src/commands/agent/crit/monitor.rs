//! Detached cleanup worker for crit review daemons.

use std::fs::{self, File, OpenOptions};
use std::os::raw::c_int;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use clap::Args;

use super::super::store;
use super::super::types::{Session, TMUX_CRIT_OPTION};
use super::{parse_port, rerun_window_layout_hook};
use crate::infra::external_tool::ExternalTool;
use crate::infra::process;
use crate::infra::tmux;

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

    /// Agent session ID that owns the review.
    #[arg(long)]
    session: String,

    /// Pane associated when the review was requested.
    #[arg(long)]
    pane: Option<String>,

    /// Marker used to prevent duplicate monitor processes for one daemon.
    #[arg(long)]
    marker: PathBuf,

    /// Reservation marker claimed by the process that launched this worker.
    #[arg(long)]
    reservation: String,
}

pub(super) fn ensure_started(port: u16, session: &Session) -> Result<()> {
    let marker = marker_path(port)?;
    let Some(reservation) = claim_marker(&marker)? else {
        return Ok(());
    };

    let result = (|| {
        let pid = crit_pid_for_port(port)?
            .with_context(|| format!("crit status did not report a daemon on port {port}"))?;
        let executable =
            std::env::current_exe().context("failed to resolve the armyknife executable")?;
        let pane_id = session
            .tmux_info
            .as_ref()
            .map(|info| info.pane_id.as_str())
            .unwrap_or_default();
        let mut args = vec![
            "agent".to_string(),
            "crit".to_string(),
            "monitor".to_string(),
            format!("--port={port}"),
            format!("--pid={pid}"),
            format!("--session={}", session.session_id),
            format!("--marker={}", marker.display()),
            format!("--reservation={reservation}"),
        ];
        if !pane_id.is_empty() {
            args.push(format!("--pane={pane_id}"));
        }

        let worker_pid =
            process::spawn_detached_with_pid(&executable, args, Some(Path::new("/")), &[])
                .context("failed to spawn crit lifecycle monitor")?;
        tracing::info!(port, worker_pid, session_id = %session.session_id, "started crit lifecycle monitor");
        Ok(())
    })();

    if result.is_err() {
        remove_marker_if_equals(&marker, &reservation);
    }
    result
}

pub(super) fn run(args: &MonitorArgs) -> Result<()> {
    write_marker_owner(&args.marker, &args.reservation)?;
    let mut watched_pid = args.pid;
    loop {
        while process_is_alive(watched_pid) {
            thread::sleep(MONITOR_POLL_INTERVAL);
        }

        match crit_pid_for_port(args.port) {
            Ok(Some(current_pid))
                if current_pid != watched_pid && process_is_alive(current_pid) =>
            {
                watched_pid = current_pid;
            }
            Ok(_) => break,
            Err(error) => {
                tracing::warn!(port = args.port, %error, "failed to check for a replacement crit daemon");
                break;
            }
        }
    }

    kill_shpool_session(args.port);
    cleanup_session_links(args.port, &args.session, args.pane.as_deref());
    remove_marker_if_owned(&args.marker);
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
    Ok(status
        .get("sessions")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .find(|session| {
            session.get("port").and_then(serde_json::Value::as_u64) == Some(port.into())
        })
        .and_then(|session| session.get("pid").and_then(serde_json::Value::as_u64))
        .and_then(|pid| u32::try_from(pid).ok()))
}

fn marker_path(port: u16) -> Result<PathBuf> {
    let sessions_dir = store::sessions_dir()?;
    let cc_dir = sessions_dir
        .parent()
        .context("session directory has no parent")?;
    Ok(cc_dir.join("crit").join(format!("{port}.monitor")))
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
    if let Ok(pid) = contents.trim().parse::<u32>() {
        return Ok(process_is_alive(pid));
    }
    let Some((pid, started_at)) = contents
        .trim()
        .strip_prefix("starting:")
        .and_then(|value| value.split_once(':'))
    else {
        return Ok(false);
    };
    let (Ok(pid), Ok(started_at)) = (pid.parse::<u32>(), started_at.parse::<u64>()) else {
        return Ok(false);
    };
    Ok(process_is_alive(pid)
        && unix_time_secs().saturating_sub(started_at) < STARTING_MARKER_TTL.as_secs())
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
        Ok(status) => tracing::debug!(port, %status, "shpool session was already stopped"),
        Err(error) => tracing::warn!(port, %error, "failed to stop crit shpool session"),
    }
}

fn cleanup_session_links(port: u16, fallback_session_id: &str, fallback_pane_id: Option<&str>) {
    let mut fallback_was_processed = false;
    match store::list_all_sessions() {
        Ok(sessions) => {
            for session in sessions.into_iter().filter(|session| {
                session
                    .crit_urls
                    .iter()
                    .any(|url| parse_port(url).ok() == Some(port))
            }) {
                fallback_was_processed |= session.session_id == fallback_session_id;
                let pane_id = session.tmux_info.as_ref().map(|info| info.pane_id.as_str());
                remove_session_link(port, &session.session_id, pane_id);
            }
        }
        Err(error) => {
            tracing::warn!(port, %error, "failed to list sessions during crit cleanup");
        }
    }
    if !fallback_was_processed {
        remove_session_link(port, fallback_session_id, fallback_pane_id);
    }
}

fn remove_session_link(port: u16, session_id: &str, fallback_pane_id: Option<&str>) {
    let sessions_dir = match store::sessions_dir() {
        Ok(dir) => dir,
        Err(error) => {
            tracing::warn!(session_id, port, %error, "failed to resolve session store during crit cleanup");
            clear_pane_if_matching(port, fallback_pane_id);
            return;
        }
    };
    let lock = match store::lock_session_for_update(&sessions_dir, session_id) {
        Ok(lock) => lock,
        Err(error) => {
            tracing::warn!(session_id, port, %error, "failed to lock session during crit cleanup");
            clear_pane_if_matching(port, fallback_pane_id);
            return;
        }
    };
    let session = match lock.load() {
        Ok(Some(session)) => session,
        Ok(None) => {
            clear_pane_if_matching(port, fallback_pane_id);
            return;
        }
        Err(error) => {
            tracing::warn!(session_id, port, %error, "failed to load session during crit cleanup");
            clear_pane_if_matching(port, fallback_pane_id);
            return;
        }
    };

    let mut updated = session;
    let previous_count = updated.crit_urls.len();
    updated
        .crit_urls
        .retain(|url| parse_port(url).ok() != Some(port));
    if updated.crit_urls.len() != previous_count
        && let Err(error) = lock.save(&updated)
    {
        tracing::warn!(session_id, port, %error, "failed to save crit link cleanup");
    }

    let pane_id = updated
        .tmux_info
        .as_ref()
        .map(|info| info.pane_id.as_str())
        .or(fallback_pane_id);
    if let Some(pane_id) = pane_id {
        if let Some(latest_url) = updated.crit_urls.last() {
            if let Err(error) = tmux::set_pane_option(pane_id, TMUX_CRIT_OPTION, latest_url) {
                tracing::warn!(session_id, pane_id, %error, "failed to restore remaining crit link");
            } else if let Err(error) = rerun_window_layout_hook(pane_id) {
                tracing::warn!(session_id, pane_id, %error, "failed to re-evaluate the tmux pane border");
            }
        } else {
            clear_pane_if_matching(port, Some(pane_id));
        }
    }
}

fn clear_pane_if_matching(port: u16, pane_id: Option<&str>) {
    let Some(pane_id) = pane_id else {
        return;
    };
    if tmux::get_pane_option(pane_id, TMUX_CRIT_OPTION)
        .as_deref()
        .and_then(|url| parse_port(url).ok())
        == Some(port)
    {
        let _ = tmux::run_tmux(&["set-option", "-p", "-u", "-t", pane_id, TMUX_CRIT_OPTION]);
        let _ = rerun_window_layout_hook(pane_id);
    }
}

fn process_is_alive(pid: u32) -> bool {
    let Ok(pid) = c_int::try_from(pid) else {
        return false;
    };
    // The worker only tracks local crit processes owned by the same user.
    unsafe { libc::kill(pid, 0) == 0 }
}

fn unix_time_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}
