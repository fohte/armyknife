//! Detects whether a tmux pane's process tree already has a live agent
//! (Claude Code or Codex) process running.

use crate::commands::agent::types::Engine;
use crate::infra::process::ProcessSnapshot;

/// Bound for the descendant walk that resolves whether an agent process
/// is already running in a pane. Same value `auto_compact::schedule` and
/// `sweep` use; a shell hosting an agent has at most a handful of children.
const MAX_DESCENDANT_NODES: usize = 64;

/// Returns whether `pane_pid`'s process tree -- the pane's own process or
/// any descendant -- currently has a running process for `engine`.
///
/// Fails closed: an unavailable `snapshot` (e.g. `ps` failed) is treated as
/// "has a live process" so callers do not mistake an unknown process tree for
/// an idle pane. Callers that need positive confirmation must handle snapshot
/// failure before calling this function.
pub fn pane_has_live_agent_process(
    pane_pid: u32,
    engine: Engine,
    snapshot: Option<&ProcessSnapshot>,
) -> bool {
    let Some(snapshot) = snapshot else {
        return true;
    };
    snapshot
        .find_self_or_descendant_by_command(pane_pid, engine.process_name(), MAX_DESCENDANT_NODES)
        .is_some()
}
