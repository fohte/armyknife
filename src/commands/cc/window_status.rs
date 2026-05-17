use std::io::{self, Write};

use anyhow::Result;
use clap::Args;

use super::store;
use super::types::{Session, SessionStatus};
use crate::infra::tmux;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct WindowStatusArgs {
    /// Tmux window ID to inspect (e.g. `@3`)
    pub window_id: String,
}

/// Runs the window-status command.
///
/// Prints the colored status symbols of every Claude Code session running in
/// the panes of the given tmux window, intended to be embedded in tmux's
/// `window-status-format` via `#(a cc window-status #{window_id})`.
pub fn run(args: &WindowStatusArgs) -> Result<()> {
    let pane_ids = tmux::list_window_pane_ids(&args.window_id);
    if pane_ids.is_empty() {
        return Ok(());
    }

    let sessions = store::list_sessions()?;

    let mut stdout = io::stdout().lock();
    render_window_status(&mut stdout, &sessions, &pane_ids)?;

    Ok(())
}

/// Renders the status symbols for a window's panes to the given writer.
///
/// For each pane (in pane order), every Claude Code session whose pane matches
/// contributes one colored symbol. Symbols are concatenated without a
/// separator and, when at least one symbol is emitted, a trailing space is
/// added so the result reads cleanly when prepended to a window name.
///
/// Separated from `run()` so the rendering logic can be tested without tmux.
fn render_window_status<W: Write>(
    writer: &mut W,
    sessions: &[Session],
    pane_ids: &[String],
) -> Result<()> {
    let mut symbols = String::new();

    for pane_id in pane_ids {
        for session in sessions {
            let in_pane = session
                .tmux_info
                .as_ref()
                .is_some_and(|info| info.pane_id == *pane_id);
            if in_pane && let Some(symbol) = format_window_symbol(session.status) {
                symbols.push_str(&symbol);
            }
        }
    }

    if !symbols.is_empty() {
        write!(writer, "{symbols} ")?;
    }

    Ok(())
}

/// Formats a single status symbol with tmux style markup.
///
/// Returns `None` only for `Ended` sessions (Claude Code fully exited).
/// `Stopped` and `Paused` are still shown: their panes are alive and the
/// conversation is resumable, which is exactly what the window indicator is
/// for. Colors follow the same scheme as `list::format_status`: running is
/// green, waiting-input is yellow, stopped is gray, paused is dim.
fn format_window_symbol(status: SessionStatus) -> Option<String> {
    let style = match status {
        SessionStatus::Running => "#[fg=green]",
        SessionStatus::WaitingInput => "#[fg=yellow]",
        SessionStatus::Stopped => "#[fg=brightblack]",
        SessionStatus::Paused => "#[dim]",
        SessionStatus::Ended => return None,
    };
    Some(format!("{style}{}#[default]", status.display_symbol()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::types::TmuxInfo;
    use chrono::Utc;
    use rstest::rstest;
    use std::path::PathBuf;

    fn session_in_pane(status: SessionStatus, pane_id: Option<&str>) -> Session {
        Session {
            session_id: "test-123".to_string(),
            cwd: PathBuf::from("/tmp/test"),
            transcript_path: None,
            tty: None,
            tmux_info: pane_id.map(|id| TmuxInfo {
                session_name: "work".to_string(),
                window_name: "editor".to_string(),
                window_index: 0,
                pane_id: id.to_string(),
            }),
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

    fn render(sessions: &[Session], pane_ids: &[&str]) -> String {
        let pane_ids: Vec<String> = pane_ids.iter().map(|s| s.to_string()).collect();
        let mut output = Vec::new();
        render_window_status(&mut output, sessions, &pane_ids).expect("render should succeed");
        String::from_utf8(output).expect("valid utf8")
    }

    #[rstest]
    #[case::running(SessionStatus::Running, Some("#[fg=green]\u{25cf}#[default]"))]
    #[case::waiting(SessionStatus::WaitingInput, Some("#[fg=yellow]\u{25d0}#[default]"))]
    #[case::stopped(SessionStatus::Stopped, Some("#[fg=brightblack]\u{25cb}#[default]"))]
    #[case::paused(SessionStatus::Paused, Some("#[dim]\u{23f8}#[default]"))]
    #[case::ended(SessionStatus::Ended, None)]
    fn test_format_window_symbol(#[case] status: SessionStatus, #[case] expected: Option<&str>) {
        assert_eq!(format_window_symbol(status).as_deref(), expected);
    }

    #[test]
    fn test_render_no_panes() {
        let sessions = vec![session_in_pane(SessionStatus::Running, Some("%0"))];
        assert_eq!(render(&sessions, &[]), "");
    }

    #[test]
    fn test_render_no_matching_session() {
        // Session lives in a pane outside this window.
        let sessions = vec![session_in_pane(SessionStatus::Running, Some("%9"))];
        assert_eq!(render(&sessions, &["%0", "%1"]), "");
    }

    #[test]
    fn test_render_session_without_tmux_info() {
        let sessions = vec![session_in_pane(SessionStatus::Running, None)];
        assert_eq!(render(&sessions, &["%0"]), "");
    }

    #[test]
    fn test_render_single_session_has_trailing_space() {
        let sessions = vec![session_in_pane(SessionStatus::Running, Some("%0"))];
        assert_eq!(render(&sessions, &["%0"]), "#[fg=green]\u{25cf}#[default] ");
    }

    #[test]
    fn test_render_multiple_sessions_in_one_window_concatenated() {
        // running / waiting / stopped sessions across the window's panes are
        // emitted individually, in pane order, with no separator.
        let sessions = vec![
            session_in_pane(SessionStatus::Running, Some("%0")),
            session_in_pane(SessionStatus::WaitingInput, Some("%1")),
            session_in_pane(SessionStatus::Stopped, Some("%2")),
        ];
        assert_eq!(
            render(&sessions, &["%0", "%1", "%2"]),
            "#[fg=green]\u{25cf}#[default]#[fg=yellow]\u{25d0}#[default]#[fg=brightblack]\u{25cb}#[default] "
        );
    }

    #[test]
    fn test_render_multiple_sessions_in_same_pane() {
        let sessions = vec![
            session_in_pane(SessionStatus::Running, Some("%0")),
            session_in_pane(SessionStatus::WaitingInput, Some("%0")),
        ];
        assert_eq!(
            render(&sessions, &["%0"]),
            "#[fg=green]\u{25cf}#[default]#[fg=yellow]\u{25d0}#[default] "
        );
    }

    #[test]
    fn test_render_follows_pane_order() {
        let sessions = vec![
            session_in_pane(SessionStatus::Stopped, Some("%2")),
            session_in_pane(SessionStatus::Running, Some("%0")),
        ];
        // Output order tracks the pane_ids slice, not the sessions slice.
        assert_eq!(
            render(&sessions, &["%0", "%2"]),
            "#[fg=green]\u{25cf}#[default]#[fg=brightblack]\u{25cb}#[default] "
        );
    }

    #[test]
    fn test_render_skips_ended_sessions() {
        let sessions = vec![
            session_in_pane(SessionStatus::Ended, Some("%0")),
            session_in_pane(SessionStatus::Running, Some("%1")),
        ];
        assert_eq!(
            render(&sessions, &["%0", "%1"]),
            "#[fg=green]\u{25cf}#[default] "
        );
    }

    #[test]
    fn test_render_only_ended_session_emits_nothing() {
        let sessions = vec![session_in_pane(SessionStatus::Ended, Some("%0"))];
        assert_eq!(render(&sessions, &["%0"]), "");
    }
}
