use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use super::super::url::parse_port;
use crate::commands::agent::store;
use crate::commands::agent::types::Session;
use crate::infra::external_tool::ExternalTool;
use crate::infra::process;

const CRIT_CLIENT_CHECK_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) fn crit_client_waiting_on_port(
    port: u16,
    daemon_pid: u32,
    run_with_timeout: impl FnOnce(
        std::process::Command,
        Duration,
    ) -> std::io::Result<std::process::Output>,
) -> Result<bool> {
    let output = run_with_timeout(crit_client_lsof_command(port), CRIT_CLIENT_CHECK_TIMEOUT)
        .context("failed to inspect crit client connections")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    crit_client_waiting_from_lsof_result(
        output.status.success(),
        output.status.code(),
        &stdout,
        &stderr,
        daemon_pid,
    )
}

fn crit_client_lsof_command(port: u16) -> std::process::Command {
    let mut command = ExternalTool::Lsof.command();
    // `-a -c crit` keeps lsof from inspecting every process's fds on each poll.
    command.args([
        "-nP",
        "-a",
        "-c",
        "crit",
        &format!("-iTCP:{port}"),
        "-sTCP:ESTABLISHED",
        "-Fpc",
    ]);
    command
}

fn crit_client_waiting_from_lsof_result(
    status_success: bool,
    exit_code: Option<i32>,
    stdout: &str,
    stderr: &str,
    daemon_pid: u32,
) -> Result<bool> {
    if !status_success {
        // lsof exits 1 with no output when no socket matches, so no client is waiting.
        if exit_code == Some(1) && stdout.is_empty() && stderr.is_empty() {
            return Ok(false);
        }
        bail!("lsof failed to inspect crit client connections: {stderr}");
    }
    has_crit_client(stdout, daemon_pid)
}

fn has_crit_client(output: &str, daemon_pid: u32) -> Result<bool> {
    let mut current_pid = None;
    let mut current_command = None;
    for field in output.lines() {
        if let Some(pid) = field.strip_prefix('p') {
            if let Some(pid) = current_pid {
                anyhow::ensure!(current_command.is_some(), "lsof omitted a process command");
                if pid != daemon_pid && current_command == Some("crit") {
                    return Ok(true);
                }
            }
            current_pid = Some(
                pid.parse::<u32>()
                    .context("lsof returned an invalid process ID")?,
            );
            current_command = None;
        } else if let Some(command) = field.strip_prefix('c') {
            anyhow::ensure!(
                current_pid.is_some(),
                "lsof returned a command without a process"
            );
            current_command = Some(command);
        }
    }
    if let Some(pid) = current_pid {
        anyhow::ensure!(current_command.is_some(), "lsof omitted a process command");
        return Ok(pid != daemon_pid && current_command == Some("crit"));
    }
    Ok(false)
}

pub(super) fn set_crit_wait_state(port: u16, waiting: Option<bool>) -> Result<()> {
    set_crit_wait_state_in(&store::sessions_dir()?, port, waiting)
}

pub(super) fn reconcile_registered_session(
    port: u16,
    session_id: &str,
    status: Option<&serde_json::Value>,
) -> Result<()> {
    reconcile_registered_session_with(
        &store::sessions_dir()?,
        session_id,
        port,
        status,
        super::status::crit_status,
        |port, daemon_pid| crit_client_waiting_on_port(port, daemon_pid, process::run_with_timeout),
    )
}

fn reconcile_registered_session_with(
    sessions_dir: &Path,
    session_id: &str,
    port: u16,
    status: Option<&serde_json::Value>,
    read_status: impl FnOnce() -> Result<serde_json::Value>,
    check_client: impl FnOnce(u16, u32) -> Result<bool>,
) -> Result<()> {
    let crit_pid = match status {
        Some(status) => Ok(super::status::pid_for_port(status, port)),
        None => read_status().map(|status| super::status::pid_for_port(&status, port)),
    };
    let waiting = match crit_pid
        .and_then(|pid| {
            pid.with_context(|| format!("crit status did not report a daemon on port {port}"))
        })
        .and_then(|pid| check_client(port, pid))
    {
        Ok(waiting) => Some(waiting),
        Err(error) => {
            tracing::warn!(
                event = "agent.crit.monitor.client_check_failed",
                port,
                error = %error,
            );
            None
        }
    };

    set_crit_wait_state_for_session(sessions_dir, session_id, port, waiting)
}

pub(super) fn set_crit_wait_state_if_changed(
    previous_waiting: &mut Option<Option<bool>>,
    waiting: Option<bool>,
    update: impl FnOnce(Option<bool>) -> Result<()>,
) -> Result<()> {
    if *previous_waiting == Some(waiting) {
        return Ok(());
    }

    update(waiting)?;
    *previous_waiting = Some(waiting);
    Ok(())
}

fn set_crit_wait_state_in(sessions_dir: &Path, port: u16, waiting: Option<bool>) -> Result<()> {
    let sessions = store::list_all_sessions_from(sessions_dir)?;
    for session_id in sessions
        .iter()
        .filter(|session| {
            session
                .crit_urls
                .iter()
                .any(|url| parse_port(url).ok() == Some(port))
        })
        .map(|session| session.session_id.as_str())
    {
        let lock = store::lock_session_for_update(sessions_dir, session_id)?;
        let Some(mut session) = lock.load()? else {
            continue;
        };
        if set_session_waiting_state(&mut session, port, waiting) {
            lock.save(&session)?;
        }
    }
    Ok(())
}

fn set_crit_wait_state_for_session(
    sessions_dir: &Path,
    session_id: &str,
    port: u16,
    waiting: Option<bool>,
) -> Result<()> {
    let lock = store::lock_session_for_update(sessions_dir, session_id)?;
    let Some(mut session) = lock.load()? else {
        return Ok(());
    };
    if set_session_waiting_state(&mut session, port, waiting) {
        lock.save(&session)?;
    }
    Ok(())
}

fn set_session_waiting_state(session: &mut Session, port: u16, waiting: Option<bool>) -> bool {
    let mut changed = false;
    for url in session
        .crit_urls
        .iter()
        .filter(|url| parse_port(url).ok() == Some(port))
    {
        match waiting {
            Some(true) | None => changed |= session.crit_not_waiting_urls.remove(url),
            Some(false) => changed |= session.crit_not_waiting_urls.insert(url.clone()),
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use rstest::rstest;

    #[test]
    fn crit_client_lsof_command_uses_supported_arguments() {
        let args = std::cell::RefCell::new(Vec::new());
        let result = crit_client_waiting_on_port(43127, 101, |command, _| {
            *args.borrow_mut() = command
                .get_args()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect();
            Err(std::io::Error::other(
                "test stops after capturing arguments",
            ))
        });

        assert_eq!(
            (args.into_inner(), result.is_err()),
            (
                vec![
                    "-nP".to_owned(),
                    "-a".to_owned(),
                    "-c".to_owned(),
                    "crit".to_owned(),
                    "-iTCP:43127".to_owned(),
                    "-sTCP:ESTABLISHED".to_owned(),
                    "-Fpc".to_owned(),
                ],
                true,
            ),
        );
    }

    #[rstest]
    #[case::waiting_client(indoc! {"
        p101
        ccrit
        p202
        ccrit
    "}, 101, true)]
    #[case::daemon_only(indoc! {"
        p101
        ccrit
    "}, 101, false)]
    #[case::browser_only(indoc! {"
        p202
        cterminal-browser
    "}, 101, false)]
    #[case::no_connections("", 101, false)]
    fn crit_client_detection_ignores_the_daemon_and_other_processes(
        #[case] lsof_output: &str,
        #[case] daemon_pid: u32,
        #[case] expected: bool,
    ) {
        assert_eq!(
            has_crit_client(lsof_output, daemon_pid).expect("fixture output should parse"),
            expected,
        );
    }

    #[test]
    fn crit_client_detection_rejects_unparseable_lsof_output() {
        assert!(
            has_crit_client(
                indoc! {"
                    pnot-a-pid
                    ccrit
                "},
                101,
            )
            .is_err()
        );
    }

    #[rstest]
    #[case::empty_result_is_successful(true, Some(0))]
    #[case::no_matching_socket_returns_nonzero(false, Some(1))]
    fn no_lsof_matches_mean_no_crit_client(
        #[case] status_success: bool,
        #[case] exit_code: Option<i32>,
    ) {
        let actual = crit_client_waiting_from_lsof_result(status_success, exit_code, "", "", 101)
            .map(|waiting| (true, waiting))
            .unwrap_or((false, false));

        assert_eq!(actual, (true, false));
    }

    #[rstest]
    #[case::diagnostics("", "lsof could not inspect a socket")]
    #[case::partial_output("partial lsof output", "")]
    fn lsof_errors_or_partial_results_remain_unknown(#[case] stdout: &str, #[case] stderr: &str) {
        let actual = crit_client_waiting_from_lsof_result(false, Some(1), stdout, stderr, 101)
            .map_err(|_| "lsof error");

        assert_eq!(actual, Err("lsof error"));
    }

    #[test]
    fn crit_wait_state_updates_only_on_first_observation_or_change() -> Result<()> {
        let mut previous_waiting = None;
        let mut updates = Vec::new();
        for waiting in [None, None, Some(false), Some(false), Some(true), Some(true)] {
            set_crit_wait_state_if_changed(&mut previous_waiting, waiting, |waiting| {
                updates.push(waiting);
                Ok(())
            })?;
        }

        assert_eq!(
            (updates, previous_waiting),
            (vec![None, Some(false), Some(true)], Some(Some(true))),
        );
        Ok(())
    }

    #[test]
    fn failed_crit_wait_state_update_retries_the_same_observation() {
        let mut previous_waiting = None;
        let mut calls = 0;
        let first = set_crit_wait_state_if_changed(&mut previous_waiting, Some(false), |_| {
            calls += 1;
            bail!("temporary storage failure")
        });
        let failed_state = previous_waiting;
        let second = set_crit_wait_state_if_changed(&mut previous_waiting, Some(false), |_| {
            calls += 1;
            Ok(())
        });

        assert_eq!(
            (
                first.is_err(),
                failed_state,
                second.is_ok(),
                calls,
                previous_waiting,
            ),
            (true, None, true, 2, Some(Some(false))),
        );
    }

    #[test]
    fn failed_client_detection_restores_the_unknown_waiting_state() -> Result<()> {
        let sessions_dir = tempfile::TempDir::new()?;
        let session: Session = serde_json::from_value(serde_json::json!({
            "session_id": "fixture-session",
            "cwd": "/fixture-worktree",
            "status": "waiting_input",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "crit_urls": ["http://localhost:48761/crit/review"]
        }))?;
        store::save_session_to(sessions_dir.path(), &session)?;
        set_crit_wait_state_in(sessions_dir.path(), 48761, Some(false))?;
        set_crit_wait_state_in(sessions_dir.path(), 48761, None)?;
        let updated = store::load_session_from(sessions_dir.path(), &session.session_id)?
            .context("session should remain available")?;

        assert_eq!(
            (
                updated.is_waiting_for_crit_review(),
                updated.crit_not_waiting_urls
            ),
            (true, Default::default()),
        );
        Ok(())
    }

    #[test]
    fn crit_wait_state_updates_every_session_registered_for_a_port() -> Result<()> {
        let sessions_dir = tempfile::TempDir::new()?;
        let mut first: Session = serde_json::from_value(serde_json::json!({
            "session_id": "fixture-first",
            "cwd": "/fixture-worktree",
            "status": "waiting_input",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "crit_urls": ["http://localhost:48761/crit/review"]
        }))?;
        let second: Session = serde_json::from_value(serde_json::json!({
            "session_id": "fixture-second",
            "cwd": "/another-fixture-worktree",
            "status": "waiting_input",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "crit_urls": ["http://localhost:48761/crit/review"]
        }))?;
        first
            .crit_urls
            .push("http://localhost:48762/crit/review".to_string());
        store::save_session_to(sessions_dir.path(), &first)?;
        store::save_session_to(sessions_dir.path(), &second)?;

        set_crit_wait_state_in(sessions_dir.path(), 48761, Some(false))?;
        let first_after = store::load_session_from(sessions_dir.path(), &first.session_id)?
            .context("first session should remain available")?;
        let second_after = store::load_session_from(sessions_dir.path(), &second.session_id)?
            .context("second session should remain available")?;

        assert_eq!(
            (
                first_after.is_waiting_for_crit_review(),
                first_after.crit_not_waiting_urls,
                second_after.is_waiting_for_crit_review(),
                second_after.crit_not_waiting_urls,
            ),
            (
                true,
                ["http://localhost:48761/crit/review".to_string()]
                    .into_iter()
                    .collect(),
                false,
                ["http://localhost:48761/crit/review".to_string()]
                    .into_iter()
                    .collect(),
            ),
        );
        Ok(())
    }

    #[test]
    fn active_monitor_reconciliation_uses_status_snapshot_for_registered_session() -> Result<()> {
        let sessions_dir = tempfile::TempDir::new()?;
        let registered: Session = serde_json::from_value(serde_json::json!({
            "session_id": "fixture-registered",
            "cwd": "/fixture-worktree",
            "status": "waiting_input",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "crit_urls": ["http://localhost:48761/crit/review"]
        }))?;
        let other: Session = serde_json::from_value(serde_json::json!({
            "session_id": "fixture-other",
            "cwd": "/another-fixture-worktree",
            "status": "waiting_input",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "crit_urls": ["http://localhost:48761/crit/review"]
        }))?;
        let status = serde_json::json!({
            "sessions": [{ "port": 48761, "pid": 12345 }],
        });
        let status_reads = std::cell::Cell::new(0);
        let inspected_connection = std::cell::Cell::new(None);
        store::save_session_to(sessions_dir.path(), &registered)?;
        store::save_session_to(sessions_dir.path(), &other)?;

        reconcile_registered_session_with(
            sessions_dir.path(),
            &registered.session_id,
            48761,
            Some(&status),
            || {
                status_reads.set(status_reads.get() + 1);
                Ok(serde_json::json!({
                    "sessions": [{ "port": 48761, "pid": 54321 }],
                }))
            },
            |port, pid| {
                inspected_connection.set(Some((port, pid)));
                Ok(false)
            },
        )?;
        let updated = store::load_session_from(sessions_dir.path(), &registered.session_id)?
            .context("registered session should remain available")?;
        let unchanged = store::load_session_from(sessions_dir.path(), &other.session_id)?
            .context("other session should remain available")?;

        assert_eq!(
            (
                updated.is_waiting_for_crit_review(),
                updated.crit_not_waiting_urls,
                unchanged.is_waiting_for_crit_review(),
                unchanged.crit_not_waiting_urls,
                status_reads.get(),
                inspected_connection.get(),
            ),
            (
                false,
                ["http://localhost:48761/crit/review".to_string()]
                    .into_iter()
                    .collect(),
                true,
                Default::default(),
                0,
                Some((48761, 12345)),
            ),
        );
        Ok(())
    }
}
