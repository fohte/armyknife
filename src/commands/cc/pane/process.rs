//! Detects whether a tmux pane's process tree already has a live `claude`
//! process running.

use crate::infra::process::ProcessSnapshot;
use crate::infra::tmux;

/// Bound for the descendant walk that resolves whether a `claude` process
/// is already running in a pane. Same value `auto_compact::schedule` and
/// `sweep` use; a shell hosting claude has at most a handful of children.
const MAX_DESCENDANT_NODES: usize = 64;

/// Returns whether `pane_id`'s process tree -- the pane's own process or any
/// descendant -- currently has a running `claude` process. Used to decide
/// whether it is safe to type a resume command into the pane.
///
/// Fails closed: an unresolvable pane pid or an unavailable `snapshot` (e.g.
/// `ps` failed) is treated as "has claude", not "no claude", since the risk
/// this guards against is retyping into a live conversation, not skipping a
/// resume that turns out to be safe.
pub fn pane_has_live_claude_process(pane_id: &str, snapshot: Option<&ProcessSnapshot>) -> bool {
    let Some(pane_pid) = tmux::get_pane_pid(pane_id) else {
        return true;
    };
    let Some(snapshot) = snapshot else {
        return true;
    };
    snapshot
        .find_self_or_descendant_by_command(pane_pid, "claude", MAX_DESCENDANT_NODES)
        .is_some()
}
