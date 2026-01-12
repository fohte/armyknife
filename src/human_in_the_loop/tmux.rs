/// Get the tmux pane ID where this process is running.
///
/// Returns `Some("%pane_id")` if running inside tmux, `None` otherwise.
///
/// Uses `TMUX_PANE` environment variable which is set by tmux when the pane
/// is created. This identifies the actual pane where the command was executed,
/// not the currently focused pane.
pub fn get_tmux_target() -> Option<String> {
    std::env::var("TMUX_PANE").ok()
}
