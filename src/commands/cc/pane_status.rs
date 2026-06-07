use std::io::{self, Write};
use std::path::Path;

use anyhow::Result;
use clap::Args;

use super::store;
use super::types::{SessionStatus, TMUX_PANE_STATUS_OPTION, TMUX_SESSION_OPTION};
use crate::infra::tmux;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct PaneStatusArgs {
    /// Tmux pane ID to inspect (e.g. `%17`)
    pub pane_id: String,
}

/// Runs the pane-status command.
///
/// Prints the Claude Code status symbol for the session bound to the given
/// tmux pane. The event-driven path writes the same string to the pane's
/// `@armyknife-cc-pane-status` option (see `sync_pane_option`); this command
/// exists for manual inspection.
pub fn run(args: &PaneStatusArgs) -> Result<()> {
    let rendered = render_for_pane(&args.pane_id, &store::sessions_dir()?)?;

    let mut stdout = io::stdout().lock();
    write!(stdout, "{rendered}")?;

    Ok(())
}

/// Recomputes the pane's Claude Code status symbol and writes it to the
/// pane's `@armyknife-cc-pane-status` user option. See `format_pane_symbol`
/// for which statuses contribute a symbol.
pub fn sync_pane_option(pane_id: &str, sessions_dir: &Path) -> Result<()> {
    let rendered = render_for_pane(pane_id, sessions_dir)?;

    let current = tmux::get_pane_option(pane_id, TMUX_PANE_STATUS_OPTION);
    if !pane_status_changed(current.as_deref(), &rendered) {
        return Ok(());
    }

    tmux::set_pane_option(pane_id, TMUX_PANE_STATUS_OPTION, &rendered)?;

    Ok(())
}

/// Loads the pane's bound session (via its `@armyknife-last-claude-code-session-id`
/// option) and renders its status symbol. Returns an empty string when the
/// pane has no session option, the session file is gone, or the status does
/// not contribute a symbol.
fn render_for_pane(pane_id: &str, sessions_dir: &Path) -> Result<String> {
    let Some(session_id) = tmux::get_pane_option(pane_id, TMUX_SESSION_OPTION) else {
        return Ok(String::new());
    };
    let Some(session) = store::load_session_from(sessions_dir, &session_id)? else {
        return Ok(String::new());
    };
    Ok(format_pane_symbol(session.status).unwrap_or("").to_string())
}

/// Formats the pane status symbol for embedding in the zsh prompt.
///
/// Returns `Some("⏸")` only for `Paused` sessions: those panes are back at
/// the zsh prompt with a resumable Claude Code conversation in the
/// background, which the indicator exists to surface. Every other status
/// returns `None` so the option holds an empty string.
fn format_pane_symbol(status: SessionStatus) -> Option<&'static str> {
    match status {
        SessionStatus::Paused => Some(SessionStatus::Paused.display_symbol()),
        _ => None,
    }
}

/// Whether the pane option must be rewritten: true when the freshly rendered
/// value differs from what tmux currently holds. An unset option (`None`) is
/// treated as an empty string.
fn pane_status_changed(current: Option<&str>, rendered: &str) -> bool {
    current.unwrap_or("") != rendered
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::running(SessionStatus::Running, None)]
    #[case::waiting(SessionStatus::WaitingInput, None)]
    #[case::stopped(SessionStatus::Stopped, None)]
    #[case::paused(SessionStatus::Paused, Some("\u{23f8}"))]
    #[case::ended(SessionStatus::Ended, None)]
    fn test_format_pane_symbol(#[case] status: SessionStatus, #[case] expected: Option<&str>) {
        assert_eq!(format_pane_symbol(status), expected);
    }

    #[rstest]
    #[case::unset_and_empty(None, "", false)]
    #[case::unset_and_nonempty(None, "\u{23f8}", true)]
    #[case::unchanged(Some("\u{23f8}"), "\u{23f8}", false)]
    #[case::cleared(Some("\u{23f8}"), "", true)]
    #[case::set_from_empty(Some(""), "\u{23f8}", true)]
    fn test_pane_status_changed(
        #[case] current: Option<&str>,
        #[case] rendered: &str,
        #[case] expected: bool,
    ) {
        assert_eq!(pane_status_changed(current, rendered), expected);
    }
}
