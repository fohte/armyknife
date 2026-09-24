//! Finds the tmux pane a hook event belongs to.
//!
//! The pane is normally reachable by walking the hook process's ancestry,
//! since the agent CLI runs inside the pane and spawns the hook as a child.
//! Codex breaks that link: when a shared `codex app-server` daemon is
//! listening on its control socket, the TUI in the pane is a thin client and
//! the agent loop -- hooks included -- runs inside the daemon, which launchd
//! started outside any pane's process tree. In that case, the session ID in
//! the hook payload is matched against the option written by the launcher.

use crate::commands::agent::types::{HookInput, TMUX_SESSION_OPTION, TMUX_SESSION_OPTION_LEGACY};
use crate::infra::tmux::{self, PaneInfo};

/// Resolves the hook's pane through process ancestry or its explicit session binding.
pub(super) fn resolve(input: &HookInput) -> Option<PaneInfo> {
    tmux::get_pane_info_by_pid(std::process::id())
        .or_else(|| find_by_session_option(&input.session_id))
}

fn find_by_session_option(session_id: &str) -> Option<PaneInfo> {
    let mut matches =
        tmux::list_all_panes_with_option(TMUX_SESSION_OPTION, TMUX_SESSION_OPTION_LEGACY)
            .into_iter()
            .filter(|pane| pane.option_value.as_deref() == Some(session_id));

    let pane = matches.next()?;
    if matches.next().is_some() {
        return None;
    }

    tmux::get_pane_info_by_pane_id(&pane.pane_id)
}
