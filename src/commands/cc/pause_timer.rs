//! Internal `a cc pause-timer` subcommand.
//!
//! This command is spawned (detached) by the Stop hook. It sleeps for the
//! configured timeout, then re-reads the session from disk and, if the session
//! is still `Stopped`, sends SIGTERM to the recorded `claude_pid` and flips the
//! status to `Paused`.
//!
//! The command is hidden from `--help`: it is an implementation detail of the
//! hook, not a user-facing entry point.

use std::path::Path;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Utc;
use clap::Args;

use super::auto_pause::{self, PauseDecision};
use super::signal::{LibcSignalSender, SignalSender};
use super::store;
use super::types::SessionStatus;
use crate::shared::config;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct PauseTimerArgs {
    /// The Claude Code session ID to pause-check.
    pub session_id: String,

    /// Override the timeout duration (for testing / manual invocation).
    /// Accepts human-friendly durations like `30s`, `10m`, `1h30m`.
    #[arg(long)]
    pub timeout: Option<String>,
}

/// Entry point for `a cc pause-timer <session_id>`.
pub fn run(args: &PauseTimerArgs) -> Result<()> {
    let config = config::load_config().unwrap_or_default();
    let timeout_str = args
        .timeout
        .clone()
        .unwrap_or_else(|| config.cc.auto_pause.timeout.clone());
    let timeout = auto_pause::parse_duration(&timeout_str)
        .with_context(|| format!("invalid cc.auto_pause.timeout `{timeout_str}`"))?;

    // Respect the enabled flag unless a manual --timeout override was given.
    if args.timeout.is_none() && !config.cc.auto_pause.enabled {
        return Ok(());
    }

    let sessions_dir = store::sessions_dir()?;
    let sender = LibcSignalSender;
    run_impl(&sessions_dir, &args.session_id, timeout, &sender, |d| {
        thread::sleep(d)
    })
}

/// Testable core. `sleep_fn` lets tests substitute the real sleep.
fn run_impl<S, F>(
    sessions_dir: &Path,
    session_id: &str,
    timeout: Duration,
    sender: &S,
    sleep_fn: F,
) -> Result<()>
where
    S: SignalSender,
    F: FnOnce(Duration),
{
    // Sleep first. If the user resumes the session during the sleep, the
    // reload below will observe the new status and we exit without killing.
    sleep_fn(timeout);

    let Some(mut session) = store::load_session_from(sessions_dir, session_id)? else {
        // Session file is gone (e.g., session ended). Nothing to do.
        return Ok(());
    };

    let decision = auto_pause::decide_pause(&session, Utc::now(), timeout);
    match decision {
        PauseDecision::Pause => {
            // Unwrap is safe: decide_pause returns NoPid when pid is None.
            let pid = session.claude_pid.unwrap_or(0);
            if pid == 0 {
                return Ok(());
            }

            // Best-effort SIGTERM. ESRCH (process already gone) is not fatal.
            if let Err(e) = sender.send(pid, libc::SIGTERM)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                eprintln!("[armyknife] warning: failed to SIGTERM pid {pid}: {e}");
            }

            session.status = SessionStatus::Paused;
            session.updated_at = Utc::now();
            store::save_session_to(sessions_dir, &session)?;
        }
        PauseDecision::NotStopped | PauseDecision::NotYetElapsed | PauseDecision::NoPid => {
            // User resumed, or timer was spawned too early, or we never knew
            // the pid. Nothing to do.
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::{DateTime, TimeDelta, Utc};
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::super::signal::test_support::RecordingSender;
    use super::super::store::save_session_to;
    use super::super::types::{Session, SessionStatus};
    use super::*;

    struct TestDir {
        #[expect(dead_code, reason = "kept alive until test ends")]
        temp: TempDir,
        path: PathBuf,
    }

    #[fixture]
    fn test_dir() -> TestDir {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().to_path_buf();
        TestDir { temp, path }
    }

    fn make_session(id: &str, status: SessionStatus, updated_at: DateTime<Utc>) -> Session {
        Session {
            session_id: id.to_string(),
            cwd: PathBuf::from("/tmp"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status,
            created_at: updated_at,
            updated_at,
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            claude_pid: Some(4242),
        }
    }

    #[rstest]
    fn pauses_elapsed_stopped_session(test_dir: TestDir) {
        let old = Utc::now() - TimeDelta::hours(1);
        let session = make_session("sess-a", SessionStatus::Stopped, old);
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        run_impl(
            &test_dir.path,
            "sess-a",
            Duration::from_secs(1),
            &sender,
            |_| { /* skip sleep */ },
        )
        .expect("run");

        assert_eq!(
            *sender.calls.borrow(),
            vec![(4242, libc::SIGTERM)],
            "should SIGTERM the recorded pid"
        );

        let reloaded = super::super::store::load_session_from(&test_dir.path, "sess-a")
            .expect("load")
            .expect("session exists");
        assert_eq!(reloaded.status, SessionStatus::Paused);
    }

    #[rstest]
    #[case::running(SessionStatus::Running)]
    #[case::waiting(SessionStatus::WaitingInput)]
    #[case::already_paused(SessionStatus::Paused)]
    #[case::ended(SessionStatus::Ended)]
    fn does_nothing_when_status_is_not_stopped(test_dir: TestDir, #[case] status: SessionStatus) {
        let old = Utc::now() - TimeDelta::hours(1);
        let session = make_session("sess-b", status, old);
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        run_impl(
            &test_dir.path,
            "sess-b",
            Duration::from_secs(1),
            &sender,
            |_| {},
        )
        .expect("run");

        assert!(
            sender.calls.borrow().is_empty(),
            "no SIGTERM should be sent"
        );
        let reloaded = super::super::store::load_session_from(&test_dir.path, "sess-b")
            .expect("load")
            .expect("session exists");
        // Status should be unchanged.
        assert_eq!(reloaded.status, status);
    }

    #[rstest]
    fn does_nothing_when_timeout_has_not_elapsed(test_dir: TestDir) {
        let recent = Utc::now();
        let session = make_session("sess-c", SessionStatus::Stopped, recent);
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        run_impl(
            &test_dir.path,
            "sess-c",
            Duration::from_secs(3600),
            &sender,
            |_| {},
        )
        .expect("run");

        assert!(sender.calls.borrow().is_empty());
        let reloaded = super::super::store::load_session_from(&test_dir.path, "sess-c")
            .expect("load")
            .expect("session exists");
        assert_eq!(reloaded.status, SessionStatus::Stopped);
    }

    #[rstest]
    fn missing_session_file_is_not_an_error(test_dir: TestDir) {
        let sender = RecordingSender::default();
        run_impl(
            &test_dir.path,
            "nonexistent",
            Duration::from_secs(1),
            &sender,
            |_| {},
        )
        .expect("run should succeed even if session is gone");
        assert!(sender.calls.borrow().is_empty());
    }

    #[rstest]
    fn esrch_is_tolerated_and_still_marks_paused(test_dir: TestDir) {
        let old = Utc::now() - TimeDelta::hours(1);
        let session = make_session("sess-d", SessionStatus::Stopped, old);
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        sender
            .next_error
            .borrow_mut()
            .replace(std::io::ErrorKind::NotFound);
        run_impl(
            &test_dir.path,
            "sess-d",
            Duration::from_secs(1),
            &sender,
            |_| {},
        )
        .expect("run");

        // The sender recorded the call even though it errored.
        assert_eq!(*sender.calls.borrow(), vec![(4242, libc::SIGTERM)]);

        let reloaded = super::super::store::load_session_from(&test_dir.path, "sess-d")
            .expect("load")
            .expect("session exists");
        assert_eq!(reloaded.status, SessionStatus::Paused);
    }
}
