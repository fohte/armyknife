use std::collections::HashSet;
use std::io::{self, Write};

use anyhow::Result;
use clap::Args;

use super::store;
use super::types::{Session, SessionStatus, TMUX_SESSION_OPTION};
use crate::infra::tmux;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct WindowStatusArgs {
    /// Tmux window ID to inspect (e.g. `@3`)
    pub window_id: String,
}

/// Runs the window-status command.
///
/// Prints the status symbols of every Claude Code session running in the
/// panes of the given tmux window, intended to be embedded in tmux's
/// `window-status-format` via `#(a cc window-status #{window_id})`.
///
/// Only the sessions of the target window's panes are loaded — resolved via
/// each pane's session-id option — so the cost stays O(panes in window)
/// rather than O(all sessions on disk), since this runs on every redraw.
pub fn run(args: &WindowStatusArgs) -> Result<()> {
    let session_ids = tmux::list_window_pane_options(&args.window_id, TMUX_SESSION_OPTION);

    // Two panes can carry the same session id (e.g. a split pane keeps the
    // option), which would otherwise render the session's symbol twice.
    let mut seen = HashSet::new();
    let mut sessions = Vec::with_capacity(session_ids.len());
    for session_id in &session_ids {
        if !seen.insert(session_id.as_str()) {
            continue;
        }
        if let Some(session) = store::load_session(session_id)? {
            sessions.push(session);
        }
    }

    let mut stdout = io::stdout().lock();
    render_window_status(&mut stdout, &sessions)?;

    Ok(())
}

/// Renders the status symbols for a window's sessions to the given writer.
///
/// Each session contributes one colored symbol, in the given order, with no
/// separator. When at least one symbol is emitted, a trailing space is added
/// so the result reads cleanly when prepended to a window name.
///
/// Separated from `run()` so the rendering logic can be tested without tmux.
fn render_window_status<W: Write>(writer: &mut W, sessions: &[Session]) -> Result<()> {
    let mut symbols = String::new();

    for session in sessions {
        if let Some(symbol) = format_window_symbol(session.status) {
            symbols.push_str(symbol);
        }
    }

    if !symbols.is_empty() {
        write!(writer, "{symbols} ")?;
    }

    Ok(())
}

/// Formats a single status symbol for embedding in tmux's window-status.
///
/// Returns `None` only for `Ended` sessions (Claude Code fully exited).
/// `Stopped` and `Paused` are still shown: their panes are alive and the
/// conversation is resumable, which is exactly what the window indicator is
/// for.
///
/// No tmux style markup is emitted. A foreground color here would visually
/// become a *background* color whenever the surrounding context has the
/// `reverse` attribute (a common idiom for `window-status-activity-style`),
/// painting the icon's cell with a color block that breaks out of the rest
/// of the tab. Shape alone (●/◐/○/⏸) carries the status well enough.
fn format_window_symbol(status: SessionStatus) -> Option<&'static str> {
    if status == SessionStatus::Ended {
        return None;
    }
    Some(status.display_symbol())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use rstest::rstest;
    use std::path::PathBuf;

    fn session(status: SessionStatus) -> Session {
        Session {
            session_id: "test-123".to_string(),
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
            last_bg_task_pending: false,
        }
    }

    fn render(statuses: &[SessionStatus]) -> String {
        let sessions: Vec<Session> = statuses.iter().copied().map(session).collect();
        let mut output = Vec::new();
        render_window_status(&mut output, &sessions).expect("render should succeed");
        String::from_utf8(output).expect("valid utf8")
    }

    #[rstest]
    #[case::running(SessionStatus::Running, Some("\u{25cf}"))]
    #[case::waiting(SessionStatus::WaitingInput, Some("\u{25d0}"))]
    #[case::stopped(SessionStatus::Stopped, Some("\u{25cb}"))]
    #[case::paused(SessionStatus::Paused, Some("\u{23f8}"))]
    #[case::ended(SessionStatus::Ended, None)]
    fn test_format_window_symbol(#[case] status: SessionStatus, #[case] expected: Option<&str>) {
        assert_eq!(format_window_symbol(status), expected);
    }

    #[rstest]
    #[case::empty(&[], "")]
    #[case::single_running(&[SessionStatus::Running], "\u{25cf} ")]
    #[case::paused(&[SessionStatus::Paused], "\u{23f8} ")]
    #[case::running_waiting_stopped(
        &[SessionStatus::Running, SessionStatus::WaitingInput, SessionStatus::Stopped],
        "\u{25cf}\u{25d0}\u{25cb} "
    )]
    #[case::skips_ended(
        &[SessionStatus::Ended, SessionStatus::Running],
        "\u{25cf} "
    )]
    #[case::only_ended(&[SessionStatus::Ended], "")]
    fn test_render_window_status(#[case] statuses: &[SessionStatus], #[case] expected: &str) {
        assert_eq!(render(statuses), expected);
    }
}
