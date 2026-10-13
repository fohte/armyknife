use std::io;
use std::path::Path;

use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::DefaultTerminal;

use crate::shared::command;

/// Preview a session's JSONL with `claude-history`.
pub(super) fn preview_session(
    terminal: &mut DefaultTerminal,
    jsonl_path: &Path,
) -> std::result::Result<(), PreviewError> {
    let suspend_err = suspend_terminal().err();
    // Even if suspend partially failed (raw mode disabled but alt screen
    // still active), we must still try to restore — the alternative is
    // leaving the TUI in a broken state.
    let status = if suspend_err.is_none() {
        Some(command::new("claude-history").arg(jsonl_path).status())
    } else {
        None
    };
    let restore_err = resume_terminal(terminal).err();
    combine_preview_result(suspend_err, status, restore_err)
}

/// Categorized failure from `preview_session`. `Fatal` means the terminal
/// itself is in a bad state (suspend or restore failed) — the caller must
/// bail so `ratatui::restore()` runs. `Viewer` means only the child failed,
/// so the TUI can keep going.
#[derive(Debug)]
pub(super) enum PreviewError {
    Fatal(anyhow::Error),
    Viewer(anyhow::Error),
}

impl std::fmt::Display for PreviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PreviewError::Fatal(e) | PreviewError::Viewer(e) => write!(f, "{e:#}"),
        }
    }
}

fn combine_preview_result(
    suspend_err: Option<io::Error>,
    status: Option<io::Result<std::process::ExitStatus>>,
    restore_err: Option<io::Error>,
) -> std::result::Result<(), PreviewError> {
    // A non-zero exit is a viewer failure the user should see, not just a
    // spawn failure — collapse both into the same "child error" slot.
    let child_err = status.and_then(|r| match r {
        Ok(st) if !st.success() => {
            Some(io::Error::other(format!("claude-history exited with {st}")))
        }
        Ok(_) => None,
        Err(e) => Some(e),
    });
    match (suspend_err, child_err, restore_err) {
        (None, None, None) => Ok(()),
        (Some(e), _, None) => Err(PreviewError::Fatal(
            anyhow::Error::from(e).context("failed to leave alt screen"),
        )),
        // Restore is the state that governs the next frame, so its error
        // surfaces first with the suspend error folded in.
        (Some(se), _, Some(re)) => Err(PreviewError::Fatal(anyhow::Error::from(re).context(
            format!("failed to restore terminal (suspend also failed: {se})"),
        ))),
        (None, None, Some(re)) => Err(PreviewError::Fatal(
            anyhow::Error::from(re).context("failed to restore terminal"),
        )),
        (None, Some(e), None) => Err(PreviewError::Viewer(
            anyhow::Error::from(e).context("failed to run claude-history"),
        )),
        // Restore failure surfaces first — a broken terminal is more urgent
        // than a missing viewer — with the child error folded in as context.
        (None, Some(ce), Some(re)) => Err(PreviewError::Fatal(anyhow::Error::from(re).context(
            format!("failed to restore terminal (child also failed: {ce})"),
        ))),
    }
}

fn suspend_terminal() -> io::Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}

fn resume_terminal(terminal: &mut DefaultTerminal) -> io::Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    terminal.clear()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io;

    use rstest::rstest;

    use super::{PreviewError, combine_preview_result};

    fn io_err(msg: &str) -> io::Error {
        io::Error::other(msg)
    }

    fn exit_ok() -> io::Result<std::process::ExitStatus> {
        exit_status(0)
    }

    fn exit_nonzero() -> io::Result<std::process::ExitStatus> {
        // On unix, from_raw takes a wait(2) status; 1 << 8 == exit code 1.
        exit_status(1 << 8)
    }

    fn exit_status(raw: i32) -> io::Result<std::process::ExitStatus> {
        // The repository is Unix-only (tmux + Unix APIs throughout), so no
        // Windows path.
        use std::os::unix::process::ExitStatusExt;
        Ok(std::process::ExitStatus::from_raw(raw))
    }

    /// Categorized outcome for whole-value equality on `combine_preview_result`.
    /// The tag identifies whether the caller should treat the failure as fatal
    /// (terminal broken) or viewer-only.
    #[derive(Debug, PartialEq, Eq)]
    enum ExpectedOutcome {
        Ok,
        Fatal(String),
        Viewer(String),
    }

    fn categorize(res: std::result::Result<(), PreviewError>) -> ExpectedOutcome {
        match res {
            Ok(()) => ExpectedOutcome::Ok,
            Err(PreviewError::Fatal(e)) => ExpectedOutcome::Fatal(format!("{e:#}")),
            Err(PreviewError::Viewer(e)) => ExpectedOutcome::Viewer(format!("{e:#}")),
        }
    }

    fn fatal(s: &str) -> ExpectedOutcome {
        ExpectedOutcome::Fatal(s.to_string())
    }

    fn viewer(s: &str) -> ExpectedOutcome {
        ExpectedOutcome::Viewer(s.to_string())
    }

    #[rstest]
    #[case::all_ok(None, Some(exit_ok()), None, ExpectedOutcome::Ok)]
    #[case::child_spawn_error(
        None,
        Some(Err(io_err("boom"))),
        None,
        viewer("failed to run claude-history: boom")
    )]
    #[case::child_nonzero_exit(
        None,
        Some(exit_nonzero()),
        None,
        viewer("failed to run claude-history: claude-history exited with exit status: 1")
    )]
    #[case::restore_error(
        None,
        Some(exit_ok()),
        Some(io_err("bad")),
        fatal("failed to restore terminal: bad")
    )]
    // When both child and restore fail the restore error surfaces first —
    // a broken terminal is more urgent than a missing viewer — with the
    // child error folded in as context.
    #[case::child_and_restore_error(
        None,
        Some(Err(io_err("no bin"))),
        Some(io_err("no tty")),
        fatal("failed to restore terminal (child also failed: no bin): no tty")
    )]
    // If suspend fails, callers pass status=None (no spawn happened) but
    // restore is still attempted; the suspend error surfaces as fatal.
    #[case::suspend_error_only(
        Some(io_err("suspend")),
        None,
        None,
        fatal("failed to leave alt screen: suspend")
    )]
    #[case::suspend_and_restore_error(
        Some(io_err("suspend")),
        None,
        Some(io_err("no tty")),
        fatal("failed to restore terminal (suspend also failed: suspend): no tty")
    )]
    fn test_combine_preview_result(
        #[case] suspend_err: Option<io::Error>,
        #[case] status: Option<io::Result<std::process::ExitStatus>>,
        #[case] restore_err: Option<io::Error>,
        #[case] expected: ExpectedOutcome,
    ) {
        assert_eq!(
            categorize(combine_preview_result(suspend_err, status, restore_err)),
            expected,
        );
    }
}
