//! `a cc peer wake` -- resume a paused Claude Code session from another
//! session's Bash tool, so its `SendMessage` name becomes resolvable.
//!
//! `a cc peer` can point at a session that `a cc sweep` has since paused:
//! its process has exited, so it has no entry in Claude Code's own session
//! registry (see `claude_registry`) and therefore no `SendMessage` name. `a
//! cc watch`'s TUI already knows how to respawn such a session's pane (see
//! `resume::respawn_paused_session`); this command drives the same respawn
//! from a non-interactive caller and waits for the new process to register
//! itself, so the caller gets back a name it can hand straight to
//! `SendMessage`. Unlike the TUI, it does not focus the pane (a resume
//! triggered from another session must not steal the user's tmux focus),
//! and it verifies the pane's last-known session ID matches the requested
//! one before respawning -- `a cc resume` (which the respawned pane runs)
//! resumes whatever session is recorded on the pane, not necessarily the
//! one this command was asked to wake.

use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::Args;
use thiserror::Error;

use crate::commands::cc::claude_registry;
use crate::commands::cc::error::CcError;
use crate::commands::cc::resume::{RespawnError, respawn_paused_session};
use crate::commands::cc::store;
use crate::commands::cc::types::{SessionStatus, TMUX_SESSION_OPTION};
use crate::infra::tmux;

/// How often to poll Claude Code's session registry for the resumed
/// process's name after respawning the pane.
const NAME_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// How long to wait for the resumed process to register its name before
/// giving up.
const NAME_POLL_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Args, Clone, PartialEq, Eq)]
pub struct WakeArgs {
    /// Session ID to wake -- the `session_id` from `a cc peer`
    pub session_id: String,
}

/// Runs the wake command: prints the resolved `SendMessage` name to stdout.
pub fn run(args: &WakeArgs) -> Result<()> {
    println!("{}", wake(&args.session_id)?);
    Ok(())
}

fn wake(session_id: &str) -> Result<String> {
    let session = store::load_session(session_id)?
        .ok_or_else(|| CcError::SessionNotFound(session_id.to_string()))?;

    if session.status != SessionStatus::Paused {
        return resolve_name(session_id).ok_or_else(|| {
            anyhow::anyhow!(
                "No SendMessage name available for session {session_id} (status: {})",
                session.status.display_name()
            )
        });
    }

    let tmux_info = session
        .tmux_info
        .as_ref()
        .ok_or_else(|| CcError::NoTmuxInfo(session_id.to_string()))?;
    let recorded = tmux::get_pane_option(&tmux_info.pane_id, TMUX_SESSION_OPTION);
    check_pane_matches_target(recorded.as_deref(), session_id)?;

    // Serialize concurrent wakes of the same paused session -- e.g. several
    // delegated children reporting back to the same paused parent at once
    // (the scenario `a wm new`'s envelope steers callers into). Without
    // this, two callers can both observe the pane still idle and both
    // respawn it, the second one killing the first one's freshly started
    // `claude`.
    let lock = store::lock_session_for_update(&store::sessions_dir()?, session_id)?;
    if let Some(name) = resolve_name(session_id) {
        return Ok(name);
    }
    match respawn_paused_session(&session) {
        Ok(_pane_id) => {}
        // The pane already moved past the shell prompt into `claude`
        // itself -- another wake (racing just outside this lock) or the
        // user beat us to it. Fall through to polling instead of
        // erroring; an unrelated `claude` here would just make the poll
        // below time out rather than silently succeed.
        Err(RespawnError::PaneBusy(cmd)) if cmd == "claude" => {}
        Err(e) => return Err(e).context("failed to resume the session's tmux pane"),
    }
    drop(lock);

    wait_for_name(session_id)
}

/// Error from [`check_pane_matches_target`]: the pane's recorded session
/// doesn't match the session this command was asked to wake.
#[derive(Debug, Error, PartialEq, Eq)]
enum WakeError {
    #[error(
        "pane's recorded session ({recorded:?}) does not match the requested session ({target}); refusing to resume a different session"
    )]
    PaneSessionMismatch {
        recorded: Option<String>,
        target: String,
    },
}

fn check_pane_matches_target(recorded: Option<&str>, target: &str) -> Result<(), WakeError> {
    if recorded == Some(target) {
        Ok(())
    } else {
        Err(WakeError::PaneSessionMismatch {
            recorded: recorded.map(str::to_string),
            target: target.to_string(),
        })
    }
}

fn resolve_name(session_id: &str) -> Option<String> {
    claude_registry::load_name_map().remove(session_id)
}

fn wait_for_name(session_id: &str) -> Result<String> {
    let deadline = Instant::now() + NAME_POLL_TIMEOUT;
    loop {
        if let Some(name) = resolve_name(session_id) {
            return Ok(name);
        }
        if Instant::now() >= deadline {
            bail!("Timed out waiting for session {session_id} to reappear after resume");
        }
        thread::sleep(NAME_POLL_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::matches(Some("abc"), "abc", Ok(()))]
    #[case::mismatched(
        Some("xyz"),
        "abc",
        Err(WakeError::PaneSessionMismatch {
            recorded: Some("xyz".to_string()),
            target: "abc".to_string(),
        })
    )]
    #[case::unset(
        None,
        "abc",
        Err(WakeError::PaneSessionMismatch {
            recorded: None,
            target: "abc".to_string(),
        })
    )]
    fn check_pane_matches_target_cases(
        #[case] recorded: Option<&str>,
        #[case] target: &str,
        #[case] expected: std::result::Result<(), WakeError>,
    ) {
        assert_eq!(check_pane_matches_target(recorded, target), expected);
    }
}
