use std::process::Command;

/// Get the current tmux session target for later restoration.
///
/// Returns `Some("session:window.pane")` if running inside tmux,
/// `None` otherwise.
pub fn get_tmux_target() -> Option<String> {
    if std::env::var("TMUX").is_err() {
        return None;
    }

    // Get session:window.pane in a single tmux call for consistency and performance
    let output = Command::new("tmux")
        .args([
            "display-message",
            "-p",
            "#{session_name}:#{window_index}.#{pane_index}",
        ])
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}
