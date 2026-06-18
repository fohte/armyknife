use anyhow::Result;
use chrono::Utc;
use clap::Args;

use super::store;
use super::types::TMUX_SESSION_OPTION;
use crate::infra::tmux;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct MarkReadArgs {
    /// Tmux pane ID (e.g. `%17`). Defaults to the current pane.
    #[arg(short = 't', long = "target")]
    pub pane_id: Option<String>,
}

/// Runs the mark-read command.
///
/// Resolves the Claude Code session bound to the given (or current) tmux
/// pane and marks it as read if it is in the `Stopped` state. Intended to
/// be wired into tmux's `pane-focus-in` hook so navigating to a pane in
/// tmux clears its `✱` unread glyph regardless of the path taken (TUI's
/// `f` key, native tmux keybindings, mouse, etc.).
///
/// No-op when the pane has no session id option, the session file is
/// missing, or the session is not unread `Stopped`.
pub fn run(args: &MarkReadArgs) -> Result<()> {
    let Some(session_id) = lookup_session_id(args.pane_id.as_deref()) else {
        return Ok(());
    };
    store::mark_session_read(&session_id, Utc::now())?;
    Ok(())
}

fn lookup_session_id(pane_id: Option<&str>) -> Option<String> {
    match pane_id {
        Some(id) => tmux::get_pane_option(id, TMUX_SESSION_OPTION),
        None => tmux::get_current_pane_option(TMUX_SESSION_OPTION),
    }
}
