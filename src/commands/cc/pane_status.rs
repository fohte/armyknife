use std::io::{self, Write};
use std::path::Path;

use anyhow::Result;
use clap::Args;

use super::store;
use super::types::{SessionStatus, TMUX_PANE_HAS_PAUSED_OPTION, TMUX_SESSION_OPTION};
use crate::infra::tmux;

/// Value written to `@armyknife-cc-pane-has-paused` when the pane's session
/// is Paused. Any non-empty value would do; `"1"` matches the boolean-flag
/// shape that the option name implies.
const PAUSED_FLAG: &str = "1";

#[derive(Args, Clone, PartialEq, Eq)]
pub struct HasPausedArgs {
    /// Tmux pane ID to inspect (e.g. `%17`)
    pub pane_id: String,
}

/// Runs the has-paused command.
///
/// Prints `1` when the tmux pane carries a `Paused` Claude Code session, and
/// the empty string otherwise. The event-driven path writes the same value
/// to the pane's `@armyknife-cc-pane-has-paused` option (see
/// `sync_pane_option`); this command exists for manual inspection.
pub fn run(args: &HasPausedArgs) -> Result<()> {
    let rendered = render_for_pane(&args.pane_id, &store::sessions_dir()?)?;

    let mut stdout = io::stdout().lock();
    write!(stdout, "{rendered}")?;

    Ok(())
}

/// Recomputes the pane's has-paused flag and writes it to the pane's
/// `@armyknife-cc-pane-has-paused` user option. See `paused_flag` for which
/// statuses set the flag.
pub fn sync_pane_option(pane_id: &str, sessions_dir: &Path) -> Result<()> {
    let rendered = render_for_pane(pane_id, sessions_dir)?;

    let current = tmux::get_pane_option(pane_id, TMUX_PANE_HAS_PAUSED_OPTION);
    if !pane_option_changed(current.as_deref(), &rendered) {
        return Ok(());
    }

    tmux::set_pane_option(pane_id, TMUX_PANE_HAS_PAUSED_OPTION, &rendered)?;

    Ok(())
}

/// Loads the pane's bound session (via its `@armyknife-last-claude-code-session-id`
/// option) and renders the has-paused flag. Returns an empty string when
/// the pane has no session option, the session file is gone, or the session
/// is not Paused.
fn render_for_pane(pane_id: &str, sessions_dir: &Path) -> Result<String> {
    let Some(session_id) = tmux::get_pane_option(pane_id, TMUX_SESSION_OPTION) else {
        return Ok(String::new());
    };
    let Some(session) = store::load_session_from(sessions_dir, &session_id)? else {
        return Ok(String::new());
    };
    Ok(paused_flag(session.status).unwrap_or("").to_string())
}

/// Returns `Some("1")` only for `Paused` sessions: those panes are back at
/// the zsh prompt with a resumable Claude Code conversation in the
/// background, which the indicator exists to surface. Every other status
/// returns `None` so the option holds an empty string.
///
/// A boolean flag is used rather than the session status name so the
/// indicator distinguishes "armyknife paused this session" from "user
/// pressed Ctrl-C to exit": the latter writes nothing, the former writes a
/// stable marker the prompt can key off without ambiguity.
fn paused_flag(status: SessionStatus) -> Option<&'static str> {
    match status {
        SessionStatus::Paused => Some(PAUSED_FLAG),
        _ => None,
    }
}

/// Whether the pane option must be rewritten: true when the freshly rendered
/// value differs from what tmux currently holds. An unset option (`None`) is
/// treated as an empty string.
fn pane_option_changed(current: Option<&str>, rendered: &str) -> bool {
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
    #[case::paused(SessionStatus::Paused, Some("1"))]
    #[case::ended(SessionStatus::Ended, None)]
    fn test_paused_flag(#[case] status: SessionStatus, #[case] expected: Option<&str>) {
        assert_eq!(paused_flag(status), expected);
    }

    #[rstest]
    #[case::unset_and_empty(None, "", false)]
    #[case::unset_and_nonempty(None, "1", true)]
    #[case::unchanged(Some("1"), "1", false)]
    #[case::cleared(Some("1"), "", true)]
    #[case::set_from_empty(Some(""), "1", true)]
    fn test_pane_option_changed(
        #[case] current: Option<&str>,
        #[case] rendered: &str,
        #[case] expected: bool,
    ) {
        assert_eq!(pane_option_changed(current, rendered), expected);
    }
}
