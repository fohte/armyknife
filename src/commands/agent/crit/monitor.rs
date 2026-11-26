//! Detached cleanup worker for crit review daemons.

mod registration;
mod start;
mod status;
mod wait_state;

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use clap::Args;

use self::status::{crit_pid_for_port, pid_for_port};
use super::super::store;
use super::super::types::Session;
use super::url::parse_port;
use crate::commands::agent::review_queue::{self, CRIT_KEY, LiveQueueRuntime};
use crate::infra::external_tool::ExternalTool;
use crate::infra::process;
use crate::infra::tmux;
use crate::shared::log::short_run_id;

pub(super) use self::start::{ensure_started, ensure_started_with_status};
use self::wait_state::{
    crit_client_waiting_on_port, set_crit_wait_state, set_crit_wait_state_if_changed,
};

pub(super) fn prepare_registration(port: u16, url: &str) -> (String, Option<serde_json::Value>) {
    registration::prepare_registration(port, url)
}

const MONITOR_POLL_INTERVAL: Duration = Duration::from_secs(1);
const STARTING_MARKER_TTL: Duration = Duration::from_secs(15);
const CLEANUP_RETRIES: usize = 3;
const CLEANUP_RETRY_DELAY: Duration = Duration::from_millis(500);

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

fn monitor_pid_for_port(
    port: u16,
    status: Option<&serde_json::Value>,
    read_status: impl FnOnce() -> Result<serde_json::Value>,
) -> Result<u32> {
    let pid = match status {
        Some(status) => pid_for_port(status, port),
        None => pid_for_port(&read_status()?, port),
    }
    .with_context(|| format!("crit status did not report a daemon on port {port}"))?;
    Ok(pid)
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
    let mut previous_waiting = None;
    loop {
        while process::is_process_alive(watched_pid) {
            let waiting = match crit_client_waiting_on_port(
                args.port,
                watched_pid,
                process::run_with_timeout,
            ) {
                Ok(waiting) => Some(waiting),
                Err(error) => {
                    tracing::warn!(
                        event = "agent.crit.monitor.client_check_failed",
                        port = args.port,
                        error = %error,
                    );
                    None
                }
            };
            if let Err(error) =
                set_crit_wait_state_if_changed(&mut previous_waiting, waiting, |waiting| {
                    set_crit_wait_state(args.port, waiting)
                })
            {
                tracing::warn!(
                    event = "agent.crit.monitor.wait_state_update_failed",
                    port = args.port,
                    error = %error,
                );
            }
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
    if let Err(error) =
        cleanup_session_links(args.port, args.session.as_deref(), args.pane.as_deref())
    {
        tracing::warn!(
            event = "agent.crit.cleanup.failed",
            port = args.port,
            error = %error,
        );
    }
    remove_marker_if_owned(&marker);
    tracing::info!(event = "agent.crit.monitor.exit", daemon_pid = watched_pid);
    Ok(())
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
) -> Result<()> {
    let mut updated_sessions = BTreeMap::new();
    let result = retry_cleanup_with(
        || {
            let sessions_dir = store::sessions_dir()?;
            cleanup_session_links_in(
                &sessions_dir,
                port,
                fallback_session_id,
                fallback_pane_id,
                &mut updated_sessions,
            )
        },
        thread::sleep,
        |attempt, error| {
            tracing::warn!(
                event = "agent.crit.cleanup.retrying",
                port,
                attempt,
                error = %error,
            );
        },
    );

    for (session_id, (session, fallback)) in updated_sessions {
        let pane_id = session
            .as_ref()
            .and_then(|session| session.tmux_info.as_ref())
            .map(|info| info.pane_id.as_str())
            .or(fallback.as_deref());
        restore_pane_link(port, pane_id, session.as_ref(), &session_id);
    }
    result
}

fn cleanup_session_links_in(
    sessions_dir: &Path,
    port: u16,
    fallback_session_id: Option<&str>,
    fallback_pane_id: Option<&str>,
    updated_sessions: &mut BTreeMap<String, (Option<Session>, Option<String>)>,
) -> Result<()> {
    let sessions = match store::list_all_sessions_from(sessions_dir) {
        Ok(sessions) => sessions,
        Err(error) => {
            if let Some(session_id) = fallback_session_id {
                let session = drop_port_from_session_in(sessions_dir, port, session_id)?;
                updated_sessions.insert(
                    session_id.to_string(),
                    (session, fallback_pane_id.map(str::to_string)),
                );
            }
            return Err(error);
        }
    };
    let mut targets = sessions
        .into_iter()
        .filter(|session| {
            session
                .crit_urls
                .iter()
                .any(|url| parse_port(url).ok() == Some(port))
        })
        .map(|session| {
            let fallback_pane = session
                .tmux_info
                .as_ref()
                .map(|info| info.pane_id.clone())
                .or_else(|| {
                    (Some(session.session_id.as_str()) == fallback_session_id)
                        .then(|| fallback_pane_id.map(str::to_string))
                        .flatten()
                });
            (session.session_id, fallback_pane)
        })
        .collect::<Vec<_>>();

    if let Some(fallback_session_id) = fallback_session_id
        && !targets
            .iter()
            .any(|(session_id, _)| session_id == fallback_session_id)
    {
        targets.push((
            fallback_session_id.to_string(),
            fallback_pane_id.map(str::to_string),
        ));
    }

    for (session_id, fallback_pane) in targets {
        let session = drop_port_from_session_in(sessions_dir, port, &session_id)?;
        updated_sessions.insert(session_id, (session, fallback_pane));
    }
    Ok(())
}

fn retry_cleanup_with<T>(
    mut cleanup: impl FnMut() -> Result<T>,
    mut wait: impl FnMut(Duration),
    mut on_retry: impl FnMut(usize, &anyhow::Error),
) -> Result<T> {
    let mut attempt = 0;
    loop {
        match cleanup() {
            Ok(value) => return Ok(value),
            Err(error) if attempt < CLEANUP_RETRIES => {
                attempt += 1;
                on_retry(attempt, &error);
                wait(CLEANUP_RETRY_DELAY);
            }
            Err(error) => return Err(error),
        }
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
    let previous_not_waiting_len = session.crit_not_waiting_urls.len();
    session
        .crit_not_waiting_urls
        .retain(|url| parse_port(url).ok() != Some(port));
    let previous_web_ui_len = session.web_uis.len();
    session
        .web_uis
        .retain(|web_ui| web_ui.name != "crit" || parse_port(&web_ui.url).ok() != Some(port));
    if session.crit_urls.len() != previous_len
        || session.crit_not_waiting_urls.len() != previous_not_waiting_len
        || session.web_uis.len() != previous_web_ui_len
    {
        lock.save(&session)?;
    }
    drop(lock);
    Ok(Some(session))
}

fn restore_pane_link(
    port: u16,
    pane_id: Option<&str>,
    session: Option<&Session>,
    session_id: &str,
) {
    let Some(pane_id) = pane_id else {
        return;
    };
    let runtime = session.map_or_else(
        LiveQueueRuntime::without_crit,
        LiveQueueRuntime::for_session,
    );
    // Other Crit reviews of the session keep the Crit slot of the queue.
    let result = if session.is_some_and(|session| !session.crit_urls.is_empty()) {
        review_queue::refresh(&runtime, pane_id)
    } else {
        review_queue::review_finished(&runtime, pane_id, CRIT_KEY)
    };
    if let Err(error) = result {
        tracing::warn!(
            event = "agent.crit.cleanup.pane_sync_failed",
            session = %session_id,
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::commands::agent::types::WebUiRegistration;

    #[test]
    fn uses_registered_status_snapshot_without_rereading() {
        let status = serde_json::json!({
            "sessions": [{ "port": 43210, "pid": 12345 }],
        });
        let status_reads = Cell::new(0);
        let result = monitor_pid_for_port(43210, Some(&status), || {
            status_reads.set(status_reads.get() + 1);
            Ok(serde_json::json!({
                "sessions": [{ "port": 43210, "pid": 54321 }],
            }))
        })
        .map_err(|error| format!("{error:#}"));

        assert_eq!((result, status_reads.get()), (Ok(12345), 0));
    }

    #[test]
    fn dropping_a_crit_port_only_removes_its_registration() {
        let sessions_dir = tempfile::TempDir::new().expect("session directory should be created");
        let session: Session = serde_json::from_value(serde_json::json!({
            "session_id": "fixture-session",
            "cwd": "/fixture-worktree",
            "status": "running",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "crit_urls": ["http://localhost:48761/crit/review"],
            "web_uis": [
                {
                    "name": "crit",
                    "url": "http://localhost:48761/crit/review",
                    "exposedUrl": "https://review.example.test/crit/review"
                },
                {
                    "name": "Editor",
                    "url": "http://localhost:48761/design/sample/"
                }
            ]
        }))
        .expect("session should deserialize");
        store::save_session_to(sessions_dir.path(), &session).expect("session should save");
        let updated = drop_port_from_session_in(sessions_dir.path(), 48761, &session.session_id)
            .expect("crit registration should be removed")
            .expect("session should remain available");

        assert_eq!(
            (updated.crit_urls, updated.web_uis),
            (
                Vec::<String>::new(),
                vec![WebUiRegistration {
                    name: "Editor".to_owned(),
                    url: "http://localhost:48761/design/sample/".to_owned(),
                    exposed_url: None,
                }],
            ),
        );
    }

    #[test]
    fn retries_database_cleanup_before_returning_success() {
        let attempts = Cell::new(0);
        let mut waits = Vec::new();
        let mut retries = Vec::new();
        let result = retry_cleanup_with(
            || {
                attempts.set(attempts.get() + 1);
                if attempts.get() < 3 {
                    bail!("fixture database failure");
                }
                Ok("cleaned")
            },
            |delay| waits.push(delay),
            |attempt, error| retries.push((attempt, format!("{error:#}"))),
        );

        assert_eq!(
            (
                result
                    .map(str::to_string)
                    .map_err(|error| format!("{error:#}")),
                attempts.get(),
                waits,
                retries,
            ),
            (
                Ok("cleaned".to_string()),
                3,
                vec![CLEANUP_RETRY_DELAY; 2],
                vec![
                    (1, "fixture database failure".to_string()),
                    (2, "fixture database failure".to_string()),
                ],
            ),
        );
    }

    #[test]
    fn stops_after_the_configured_number_of_database_retries() {
        let attempts = Cell::new(0);
        let mut waits = Vec::new();
        let result: Result<()> = retry_cleanup_with(
            || {
                attempts.set(attempts.get() + 1);
                bail!("fixture database failure")
            },
            |delay| waits.push(delay),
            |_, _| {},
        );

        assert_eq!(
            (
                result.map_err(|error| format!("{error:#}")),
                attempts.get(),
                waits,
            ),
            (
                Err("fixture database failure".to_string()),
                CLEANUP_RETRIES + 1,
                vec![CLEANUP_RETRY_DELAY; CLEANUP_RETRIES],
            ),
        );
    }

    #[test]
    fn cleanup_removes_matching_and_fallback_session_links_from_a_temp_store() {
        use std::collections::{BTreeMap as SessionMap, BTreeSet};
        use std::path::PathBuf;

        use chrono::Utc;
        use tempfile::TempDir;

        use crate::commands::agent::types::SessionStatus;

        fn fixture_session(id: &str, urls: &[&str], archived: bool) -> Session {
            let timestamp = Utc::now();
            Session {
                launch_id: None,
                archived_at: archived.then_some(timestamp),
                pr_context: None,
                created_pr_urls: Vec::new(),
                session_id: id.to_string(),
                crit_urls: urls.iter().map(|url| (*url).to_string()).collect(),
                crit_not_waiting_urls: Default::default(),
                web_uis: Vec::new(),
                pending_human_review_ids: BTreeSet::new(),
                cwd: PathBuf::from("/tmp/fixture-worktree"),
                transcript_path: None,
                tty: None,
                tmux_info: None,
                status: SessionStatus::Running,
                created_at: timestamp,
                updated_at: timestamp,
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
                pending_bg_task_ids: BTreeSet::new(),
                pending_agent_task_ids: BTreeSet::new(),
                pending_permission_agent_ids: BTreeSet::new(),
                pending_permission_request_ids: SessionMap::new(),
                read_at: None,
                sweep_signaled: false,
                stop_blocked_by_armyknife: false,
                agent_status: None,
                agent_status_clear_pending: false,
                engine: Default::default(),
                model: None,
                effort: None,
            }
        }

        let temp = TempDir::new().expect("temporary directory should be created");
        let sessions_dir = temp.path();
        let mut active = fixture_session(
            "fixture-active-session",
            &[
                "http://localhost:43210/review",
                "http://localhost:43211/review",
            ],
            false,
        );
        let archived_fallback = fixture_session(
            "fixture-archived-session",
            &[
                "http://localhost:43210/review",
                "http://localhost:43212/review",
            ],
            true,
        );
        active.web_uis = vec![
            crate::commands::agent::types::WebUiRegistration {
                name: "crit".to_owned(),
                url: "http://localhost:43210/review".to_owned(),
                exposed_url: None,
            },
            crate::commands::agent::types::WebUiRegistration {
                name: "Design editor".to_owned(),
                url: "http://localhost:43210/design/sample/".to_owned(),
                exposed_url: None,
            },
            crate::commands::agent::types::WebUiRegistration {
                name: "crit".to_owned(),
                url: "http://localhost:43211/review".to_owned(),
                exposed_url: None,
            },
        ];
        store::save_session_to(sessions_dir, &active).expect("active session should save");
        store::save_session_to(sessions_dir, &archived_fallback)
            .expect("archived fallback session should save");
        let mut updated_sessions = BTreeMap::new();

        cleanup_session_links_in(
            sessions_dir,
            43210,
            Some("fixture-archived-session"),
            Some("%fixture-pane"),
            &mut updated_sessions,
        )
        .expect("session links should be cleaned");

        let active_after = store::load_session_from(sessions_dir, "fixture-active-session")
            .expect("active session should load")
            .expect("active session should exist");
        let archived_after = store::load_session_from(sessions_dir, "fixture-archived-session")
            .expect("archived session should load")
            .expect("archived session should exist");

        assert_eq!(
            (
                active_after.crit_urls,
                active_after.web_uis,
                archived_after.crit_urls,
                archived_after.web_uis,
                updated_sessions.keys().cloned().collect::<Vec<_>>(),
            ),
            (
                vec!["http://localhost:43211/review".to_string()],
                vec![
                    crate::commands::agent::types::WebUiRegistration {
                        name: "Design editor".to_owned(),
                        url: "http://localhost:43210/design/sample/".to_owned(),
                        exposed_url: None,
                    },
                    crate::commands::agent::types::WebUiRegistration {
                        name: "crit".to_owned(),
                        url: "http://localhost:43211/review".to_owned(),
                        exposed_url: None,
                    },
                ],
                vec!["http://localhost:43212/review".to_string()],
                Vec::new(),
                vec![
                    "fixture-active-session".to_string(),
                    "fixture-archived-session".to_string(),
                ],
            ),
        );
    }
}
