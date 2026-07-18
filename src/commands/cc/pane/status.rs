use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Args;

use crate::commands::cc::store;
use crate::commands::cc::types::{SessionStatus, TMUX_SESSION_OPTION};
use crate::infra::tmux;

/// Filename prefix for the per-pane paused-flag file. The full path is
/// `<flag_dir>/<PAUSED_FLAG_FILE_PREFIX><user>-<pane_id>` (e.g.
/// `/tmp/armyknife-cc-paused-fohte-%17`). The `<user>` segment prevents
/// collisions on multi-user hosts where `TMPDIR` falls back to a shared
/// `/tmp` (tmux pane IDs are sequential and clash across users otherwise).
/// Existence encodes the flag; the file's content is irrelevant.
const PAUSED_FLAG_FILE_PREFIX: &str = "armyknife-cc-paused-";

/// Value printed by `a cc pane-has-paused` when the pane's session is Paused.
const PAUSED_FLAG_VALUE: &str = "1";

#[derive(Args, Clone, PartialEq, Eq)]
pub struct HasPausedArgs {
    /// Tmux pane ID to inspect (e.g. `%17`)
    pub pane_id: String,
}

/// Runs the has-paused command.
///
/// Prints `1` when the tmux pane carries a `Paused` Claude Code session, and
/// the empty string otherwise. The event-driven path writes the same signal
/// as a marker file (see `sync_paused_flag`); this command exists for manual
/// inspection.
pub fn run(args: &HasPausedArgs) -> Result<()> {
    let rendered = render_for_pane(&args.pane_id, &store::sessions_dir()?)?.unwrap_or("");

    let mut stdout = io::stdout().lock();
    write!(stdout, "{rendered}")?;

    Ok(())
}

/// Recomputes the pane's paused flag and materializes it as a marker file
/// under the process temp dir. Existence of
/// `<flag_dir>/armyknife-cc-paused-<user>-<pane_id>` means the pane's
/// session is `Paused`; absence means anything else.
///
/// Uses a file rather than a tmux user option so prompt renderers can check
/// the state with `test -e` without spawning a tmux client on every prompt.
///
/// Pass `Some(status)` when the caller already has the session in memory
/// (e.g. the hook event handler) to skip the tmux subprocess + disk read
/// that the fallback path would otherwise incur on every event.
pub fn sync_paused_flag(
    pane_id: &str,
    status: Option<SessionStatus>,
    sessions_dir: &Path,
) -> Result<()> {
    sync_paused_flag_in(
        pane_id,
        status,
        sessions_dir,
        &std::env::temp_dir(),
        &current_user(),
    )
}

fn sync_paused_flag_in(
    pane_id: &str,
    status: Option<SessionStatus>,
    sessions_dir: &Path,
    flag_dir: &Path,
    user: &str,
) -> Result<()> {
    let is_paused = match status {
        Some(s) => paused_flag(s).is_some(),
        None => render_for_pane(pane_id, sessions_dir)?.is_some(),
    };
    let path = paused_flag_path(flag_dir, user, pane_id);
    if is_paused {
        // `File::create` is idempotent w.r.t. existence: concurrent hook
        // deliveries for the same pane converge on the same terminal state
        // without needing a lock. Content is unused; prompt renderers only
        // check existence.
        std::fs::File::create(&path)?;
    } else {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

fn paused_flag_path(flag_dir: &Path, user: &str, pane_id: &str) -> PathBuf {
    flag_dir.join(format!("{PAUSED_FLAG_FILE_PREFIX}{user}-{pane_id}"))
}

fn current_user() -> String {
    std::env::var("USER").unwrap_or_else(|_| "unknown".to_string())
}

/// Loads the pane's bound session (via its `@armyknife-last-claude-code-session-id`
/// option) and renders the has-paused flag. Returns `None` when the pane
/// has no session option, the session file is gone, or the session is not
/// Paused.
fn render_for_pane(pane_id: &str, sessions_dir: &Path) -> Result<Option<&'static str>> {
    let Some(session_id) = tmux::get_pane_option(pane_id, TMUX_SESSION_OPTION) else {
        return Ok(None);
    };
    Ok(is_session_paused(sessions_dir, &session_id)?.then_some(PAUSED_FLAG_VALUE))
}

/// Returns whether the session identified by `session_id` under `sessions_dir`
/// is currently `Paused`. A missing or corrupted session file is treated as
/// not paused, since both mean there is no resumable conversation to guard.
fn is_session_paused(sessions_dir: &Path, session_id: &str) -> Result<bool> {
    let session = store::load_session_from(sessions_dir, session_id)?;
    Ok(session.is_some_and(|s| s.status == SessionStatus::Paused))
}

/// Returns `Some("1")` only for `Paused` sessions: those panes are back at
/// the zsh prompt with a resumable Claude Code conversation in the
/// background, which the indicator exists to surface. Every other status
/// returns `None`.
///
/// A boolean flag is used rather than the session status name so the
/// indicator distinguishes "armyknife paused this session" (flag file
/// exists) from "user pressed Ctrl-C to exit" (no flag file).
fn paused_flag(status: SessionStatus) -> Option<&'static str> {
    match status {
        SessionStatus::Paused => Some(PAUSED_FLAG_VALUE),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::types::Session;
    use chrono::Utc;
    use rstest::rstest;
    use tempfile::TempDir;

    const TEST_USER: &str = "tester";

    fn test_session(id: &str, status: SessionStatus) -> Session {
        Session {
            session_id: id.to_string(),
            cwd: PathBuf::from("/tmp/test"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: std::collections::BTreeSet::new(),
            pending_agent_task_ids: std::collections::BTreeSet::new(),
            read_at: None,
            sweep_signaled: false,
        }
    }

    #[rstest]
    #[case::running(SessionStatus::Running, None)]
    #[case::waiting(SessionStatus::WaitingInput, None)]
    #[case::stopped(SessionStatus::Stopped, None)]
    #[case::paused(SessionStatus::Paused, Some("1"))]
    #[case::ended(SessionStatus::Ended, None)]
    fn test_paused_flag(#[case] status: SessionStatus, #[case] expected: Option<&str>) {
        assert_eq!(paused_flag(status), expected);
    }

    #[rstest]
    #[case::running(SessionStatus::Running, false)]
    #[case::waiting(SessionStatus::WaitingInput, false)]
    #[case::stopped(SessionStatus::Stopped, false)]
    #[case::paused(SessionStatus::Paused, true)]
    #[case::ended(SessionStatus::Ended, false)]
    fn is_session_paused_matches_status(#[case] status: SessionStatus, #[case] expected: bool) {
        let sessions_dir = TempDir::new().expect("temp dir");
        let session = test_session("sess-1", status);
        store::save_session_to(sessions_dir.path(), &session).expect("save session");

        assert_eq!(
            is_session_paused(sessions_dir.path(), "sess-1").expect("should not error"),
            expected,
        );
    }

    #[test]
    fn is_session_paused_false_for_missing_session() {
        let sessions_dir = TempDir::new().expect("temp dir");

        assert!(
            !is_session_paused(sessions_dir.path(), "no-such-session").expect("should not error")
        );
    }

    #[test]
    fn paused_flag_path_shape() {
        let dir = Path::new("/tmp");
        assert_eq!(
            paused_flag_path(dir, "fohte", "%17"),
            PathBuf::from("/tmp/armyknife-cc-paused-fohte-%17"),
        );
    }

    #[rstest]
    #[case::running(SessionStatus::Running, false)]
    #[case::waiting(SessionStatus::WaitingInput, false)]
    #[case::stopped(SessionStatus::Stopped, false)]
    #[case::paused(SessionStatus::Paused, true)]
    #[case::ended(SessionStatus::Ended, false)]
    fn sync_creates_or_removes_flag_file(
        #[case] status: SessionStatus,
        #[case] should_exist_after: bool,
    ) {
        let flag_dir = TempDir::new().unwrap();
        let sessions_dir = TempDir::new().unwrap();
        let pane_id = "%42";

        sync_paused_flag_in(
            pane_id,
            Some(status),
            sessions_dir.path(),
            flag_dir.path(),
            TEST_USER,
        )
        .unwrap();

        assert_eq!(
            paused_flag_path(flag_dir.path(), TEST_USER, pane_id).exists(),
            should_exist_after,
        );
    }

    #[test]
    fn sync_clears_stale_flag_when_not_paused() {
        let flag_dir = TempDir::new().unwrap();
        let sessions_dir = TempDir::new().unwrap();
        let pane_id = "%7";
        let path = paused_flag_path(flag_dir.path(), TEST_USER, pane_id);
        std::fs::write(&path, b"stale").unwrap();

        sync_paused_flag_in(
            pane_id,
            Some(SessionStatus::Running),
            sessions_dir.path(),
            flag_dir.path(),
            TEST_USER,
        )
        .unwrap();

        assert!(!path.exists());
    }

    #[test]
    fn sync_is_idempotent_when_paused() {
        let flag_dir = TempDir::new().unwrap();
        let sessions_dir = TempDir::new().unwrap();
        let pane_id = "%9";

        for _ in 0..2 {
            sync_paused_flag_in(
                pane_id,
                Some(SessionStatus::Paused),
                sessions_dir.path(),
                flag_dir.path(),
                TEST_USER,
            )
            .unwrap();
        }

        assert!(paused_flag_path(flag_dir.path(), TEST_USER, pane_id).exists());
    }
}
