//! Finds the tmux pane a hook event belongs to.
//!
//! The pane is normally reachable by walking the hook process's ancestry,
//! since the agent CLI runs inside the pane and spawns the hook as a child.
//! Codex breaks that link: when a shared `codex app-server` daemon is
//! listening on its control socket, the TUI in the pane is a thin client and
//! the agent loop -- hooks included -- runs inside the daemon, which launchd
//! started outside any pane's process tree. Such a hook has no ancestry path
//! to the pane and carries nothing pane-specific in its payload or
//! environment, so its pane cannot be resolved here.

use crate::commands::agent::types::HookInput;
use crate::infra::tmux::{self, PaneInfo};

/// Resolves the pane containing this hook process, if its ancestry reaches one.
pub(super) fn resolve(_input: &HookInput) -> Option<PaneInfo> {
    tmux::get_pane_info_by_pid(std::process::id())
}
