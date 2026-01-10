use std::process::Command;

/// Tmux format string for getting the current target.
///
/// Uses unique IDs (`window_id` and `pane_id`) instead of indices to ensure
/// correct restoration even when windows/panes are created or deleted.
/// - `window_id`: Unique ID like `@5` (stable across window reordering)
/// - `pane_id`: Unique ID like `%10` (stable across pane reordering)
const TMUX_TARGET_FORMAT: &str = "#{session_name}:#{window_id}.#{pane_id}";

/// Get the current tmux session target for later restoration.
///
/// Returns `Some("session:@window_id.%pane_id")` if running inside tmux,
/// `None` otherwise.
///
/// Uses unique IDs (`window_id` and `pane_id`) instead of indices to ensure
/// correct restoration even when windows/panes are created or deleted.
pub fn get_tmux_target() -> Option<String> {
    if std::env::var("TMUX").is_err() {
        return None;
    }

    let output = Command::new("tmux")
        .args(["display-message", "-p", TMUX_TARGET_FORMAT])
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}
