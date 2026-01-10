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

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    /// Check if a tmux target string uses stable IDs (window_id and pane_id).
    ///
    /// Stable format: `session:@123.%456`
    /// Unstable format: `session:1.0` (uses indices that can shift)
    fn is_stable_target(target: &str) -> bool {
        // Expected format: session_name:@window_id.%pane_id
        // The @ prefix indicates window_id, % prefix indicates pane_id
        if let Some((_session, window_pane)) = target.split_once(':') {
            if let Some((window, pane)) = window_pane.split_once('.') {
                return window.starts_with('@') && pane.starts_with('%');
            }
        }
        false
    }

    #[rstest]
    #[case::stable_simple("main:@1.%2", true)]
    #[case::stable_large_ids("my-session:@123.%456", true)]
    #[case::unstable_indices("main:1.0", false)]
    #[case::unstable_mixed_window("main:1.%2", false)]
    #[case::unstable_mixed_pane("main:@1.2", false)]
    fn test_is_stable_target(#[case] target: &str, #[case] expected: bool) {
        assert_eq!(is_stable_target(target), expected);
    }

    #[rstest]
    fn test_tmux_target_format_uses_stable_ids() {
        // Verify the format string uses window_id and pane_id (not window_index/pane_index)
        assert!(
            TMUX_TARGET_FORMAT.contains("window_id"),
            "Format should use window_id, not window_index"
        );
        assert!(
            TMUX_TARGET_FORMAT.contains("pane_id"),
            "Format should use pane_id, not pane_index"
        );
        assert!(
            !TMUX_TARGET_FORMAT.contains("window_index"),
            "Format should not use window_index"
        );
        assert!(
            !TMUX_TARGET_FORMAT.contains("pane_index"),
            "Format should not use pane_index"
        );
    }
}
