//! `a agent peer wake` -- resume a paused Claude Code session from another
//! session's Bash tool, so its `SendMessage` name becomes resolvable.
//!
//! `a agent peer` can point at a session that `a agent sweep` has since paused:
//! its process has exited, so it has no entry in Claude Code's own session
//! registry (see `claude_registry`) and therefore no `SendMessage` name. `a
//! cc watch`'s TUI already knows how to respawn such a session's pane (see
//! `resume::respawn_paused_session`); this command drives the same respawn
//! from a non-interactive caller and waits for the new process to register
//! itself, so the caller gets back a name it can hand straight to
//! `SendMessage`. Unlike the TUI, it does not focus the pane (a resume
//! triggered from another session must not steal the user's tmux focus),
//! and it verifies the pane's last-known session ID matches the requested
//! one before respawning -- `a agent resume` (which the respawned pane runs)
//! resumes whatever session is recorded on the pane, not necessarily the
//! one this command was asked to wake.
//!
//! A Codex session has no `SendMessage` name to wait for, so waking one only
//! respawns the pane and returns no name. That is enough for
//! `peer::notify`: `codex queue` persists to a DB under `$CODEX_HOME`, and
//! the resumed `codex` dispatches any pending queue when it loads the thread.

use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::Args;
use thiserror::Error;

use crate::commands::agent::claude_registry;
use crate::commands::agent::error::CcError;
use crate::commands::agent::resume::{RespawnError, respawn_paused_session};
use crate::commands::agent::store;
use crate::commands::agent::types::{Engine, SessionStatus, TMUX_SESSION_OPTION};
use crate::infra::tmux;

/// How often to poll Claude Code's session registry for the resumed
/// process's name after respawning the pane.
const NAME_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// How long to wait for the resumed process to register its name before
/// giving up.
const NAME_POLL_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Args, Clone, PartialEq, Eq)]
pub struct WakeArgs {
    /// Session ID to wake -- the `session_id` from `a agent peer`
    pub session_id: String,
}

/// Runs the wake command: prints the resolved `SendMessage` name to stdout
/// (nothing for a Codex session, which has none).
pub fn run(args: &WakeArgs) -> Result<()> {
    if let Some(name) = wake(&args.session_id)? {
        println!("{name}");
    }
    Ok(())
}

/// Resumes `session_id` if paused and returns its resolved `SendMessage`
/// name, or `None` for a Codex session. `pub(super)` so `peer::notify` can
/// drive the same resume flow before delivering a message to a paused
/// session.
pub(super) fn wake(session_id: &str) -> Result<Option<String>> {
    let session = store::load_session(session_id)?
        .ok_or_else(|| CcError::SessionNotFound(session_id.to_string()))?;

    if session.status != SessionStatus::Paused {
        return match session.engine {
            Engine::Claude => resolve_name(session_id).map(Some).ok_or_else(|| {
                anyhow::anyhow!(
                    "No SendMessage name available for session {session_id} (status: {})",
                    session.status.display_name()
                )
            }),
            Engine::Codex => Ok(None),
        };
    }

    let tmux_info = session
        .tmux_info
        .as_ref()
        .ok_or_else(|| CcError::NoTmuxInfo(session_id.to_string()))?;
    let recorded = tmux::get_pane_option(&tmux_info.pane_id, TMUX_SESSION_OPTION);
    check_pane_matches_target(recorded.as_deref(), session_id)?;

    // Serialize concurrent wakes of the same paused session -- e.g. several
    // delegated children reporting back to the same paused parent at once
    // (the scenario `a agent new`'s envelope steers callers into). Without
    // this, two callers can both observe the pane still idle and both
    // respawn it, the second one killing the first one's freshly started
    // `claude`.
    let lock = store::lock_session_for_update(&store::sessions_dir()?, session_id)?;
    if let Some(name) = resolve_name(session_id) {
        return Ok(Some(name));
    }
    match respawn_paused_session(&session) {
        Ok(_pane_id) => {}
        // The pane already moved past the shell prompt into the session's
        // agent itself -- another wake (racing just outside this lock) or
        // the user beat us to it. Fall through to polling instead of
        // erroring; an unrelated process with the same name here would just
        // make the poll below time out rather than silently succeed.
        Err(RespawnError::PaneBusy(cmd)) if cmd == session.engine.process_name() => {}
        Err(e) => return Err(e).context("failed to resume the session's tmux pane"),
    }
    drop(lock);

    match session.engine {
        Engine::Claude => wait_for_name(session_id).map(Some),
        Engine::Codex => Ok(None),
    }
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
