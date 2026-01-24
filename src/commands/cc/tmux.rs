use std::process::Command;

use super::types::TmuxInfo;

/// Gets tmux pane information for a given TTY device.
/// Returns None if not running in tmux or if the TTY is not found.
pub fn get_tmux_info_from_tty(tty: &str) -> Option<TmuxInfo> {
    // Check if tmux is available and we're in a tmux session
    let output = Command::new("tmux")
        .args([
            "list-panes",
            "-a",
            "-F",
            "#{pane_tty} #{session_name} #{window_name} #{window_index} #{pane_id}",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Find the line matching our TTY
    for line in stdout.lines() {
        if let Some(info) = parse_tmux_pane_line(line, tty) {
            return Some(info);
        }
    }

    None
}

/// Parses a single line from tmux list-panes output.
/// Format: "#{pane_tty} #{session_name} #{window_name} #{window_index} #{pane_id}"
fn parse_tmux_pane_line(line: &str, target_tty: &str) -> Option<TmuxInfo> {
    let parts: Vec<&str> = line.split_whitespace().collect();

    if parts.len() < 5 {
        return None;
    }

    let pane_tty = parts[0];

    // Check if this line matches our target TTY
    if pane_tty != target_tty {
        return None;
    }

    let session_name = parts[1].to_string();
    let window_name = parts[2].to_string();
    let window_index = parts[3].parse::<u32>().ok()?;
    let pane_id = parts[4].to_string();

    Some(TmuxInfo {
        session_name,
        window_name,
        window_index,
        pane_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::standard_line(
        "/dev/ttys001 main editor 0 %0",
        "/dev/ttys001",
        Some(("main", "editor", 0, "%0"))
    )]
    #[case::different_tty("/dev/ttys002 work terminal 1 %5", "/dev/ttys001", None)]
    #[case::high_window_index(
        "/dev/pts/0 session window 99 %123",
        "/dev/pts/0",
        Some(("session", "window", 99, "%123"))
    )]
    #[case::insufficient_parts("/dev/ttys001 main", "/dev/ttys001", None)]
    #[case::empty_line("", "/dev/ttys001", None)]
    fn test_parse_tmux_pane_line(
        #[case] line: &str,
        #[case] target_tty: &str,
        #[case] expected: Option<(&str, &str, u32, &str)>,
    ) {
        let result = parse_tmux_pane_line(line, target_tty);

        match expected {
            Some((session, window, index, pane)) => {
                let info = result.expect("expected Some(TmuxInfo)");
                assert_eq!(info.session_name, session);
                assert_eq!(info.window_name, window);
                assert_eq!(info.window_index, index);
                assert_eq!(info.pane_id, pane);
            }
            None => assert!(result.is_none()),
        }
    }
}
