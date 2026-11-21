use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use super::super::url::parse_port;
use crate::commands::agent::store;
use crate::infra::external_tool::ExternalTool;
use crate::infra::process;

const CRIT_CLIENT_CHECK_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) fn crit_client_waiting_on_port(port: u16, daemon_pid: u32) -> Result<bool> {
    let mut command = ExternalTool::Lsof.command();
    command.args(["-nP", &format!("-iTCP:{port}"), "-sTCP:ESTABLISHED", "-Fpc"]);
    let output = process::run_with_timeout(command, CRIT_CLIENT_CHECK_TIMEOUT)
        .context("failed to inspect crit client connections")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("lsof failed to inspect crit client connections: {stderr}");
    }
    has_crit_client(&String::from_utf8_lossy(&output.stdout), daemon_pid)
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
        if changed {
            lock.save(&session)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::types::Session;
    use indoc::indoc;
    use rstest::rstest;

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
}
