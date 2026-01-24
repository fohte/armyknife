use anyhow::Result;
use chrono::Utc;
use clap::Args;

use super::store;
use super::types::{Session, SessionStatus};

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ListArgs {}

/// Runs the list command.
/// Displays all Claude Code sessions in a formatted table.
pub fn run(_args: &ListArgs) -> Result<()> {
    // Clean up stale sessions first
    store::cleanup_stale_sessions()?;

    // Load all sessions
    let sessions = store::list_sessions()?;

    if sessions.is_empty() {
        println!("No active Claude Code sessions.");
        return Ok(());
    }

    // Print header
    println!(
        "  {:<24} {:<20} {:<10} UPDATED",
        "SESSION", "WINDOW", "STATUS"
    );

    // Print each session
    for session in sessions {
        print_session_row(&session);
    }

    Ok(())
}

/// Prints a single session row.
fn print_session_row(session: &Session) {
    let session_name = get_session_display_name(session);
    let window_name = get_window_display_name(session);
    let status_display = format_status(session.status);
    let updated_display = format_relative_time(session.updated_at);

    println!(
        "  {:<24} {:<20} {} {:<8} {}",
        truncate(&session_name, 24),
        truncate(&window_name, 20),
        session.status.display_symbol(),
        status_display,
        updated_display
    );
}

/// Gets the display name for a session.
/// Uses tmux session name if available, otherwise the last component of cwd.
fn get_session_display_name(session: &Session) -> String {
    if let Some(ref tmux_info) = session.tmux_info {
        return tmux_info.session_name.clone();
    }

    // Extract last component of cwd path
    session
        .cwd
        .file_name()
        .and_then(|n| n.to_str())
        .map(String::from)
        .unwrap_or_else(|| session.cwd.display().to_string())
}

/// Gets the display name for the window.
/// Uses tmux window name if available, otherwise "-".
fn get_window_display_name(session: &Session) -> String {
    session
        .tmux_info
        .as_ref()
        .map(|info| info.window_name.clone())
        .unwrap_or_else(|| "-".to_string())
}

/// Formats the status with color codes.
fn format_status(status: SessionStatus) -> String {
    let name = status.display_name();

    match status {
        SessionStatus::Running => format!("\x1b[32m{name}\x1b[0m"), // Green
        SessionStatus::WaitingInput => format!("\x1b[33m{name}\x1b[0m"), // Yellow
        SessionStatus::Stopped => format!("\x1b[90m{name}\x1b[0m"), // Gray
    }
}

/// Formats a datetime as a relative time string.
fn format_relative_time(dt: chrono::DateTime<Utc>) -> String {
    let now = Utc::now();
    let duration = now.signed_duration_since(dt);

    let seconds = duration.num_seconds();
    if seconds < 0 {
        return "just now".to_string();
    }

    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;

    if seconds < 60 {
        "just now".to_string()
    } else if minutes < 60 {
        format!("{minutes}m ago")
    } else if hours < 24 {
        format!("{hours}h ago")
    } else {
        format!("{days}d ago")
    }
}

/// Truncates a string to the specified length.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::types::TmuxInfo;
    use chrono::Duration;
    use rstest::rstest;
    use std::path::PathBuf;

    fn create_test_session() -> Session {
        Session {
            session_id: "test-123".to_string(),
            cwd: PathBuf::from("/home/user/projects/myproject"),
            transcript_path: None,
            tty: Some("/dev/ttys001".to_string()),
            tmux_info: None,
            status: SessionStatus::Running,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
        }
    }

    #[test]
    fn test_get_session_display_name_without_tmux() {
        let session = create_test_session();
        assert_eq!(get_session_display_name(&session), "myproject");
    }

    #[test]
    fn test_get_session_display_name_with_tmux() {
        let mut session = create_test_session();
        session.tmux_info = Some(TmuxInfo {
            session_name: "work".to_string(),
            window_name: "editor".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        });
        assert_eq!(get_session_display_name(&session), "work");
    }

    #[test]
    fn test_get_window_display_name_without_tmux() {
        let session = create_test_session();
        assert_eq!(get_window_display_name(&session), "-");
    }

    #[test]
    fn test_get_window_display_name_with_tmux() {
        let mut session = create_test_session();
        session.tmux_info = Some(TmuxInfo {
            session_name: "work".to_string(),
            window_name: "editor".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        });
        assert_eq!(get_window_display_name(&session), "editor");
    }

    #[rstest]
    #[case::just_now(0, "just now")]
    #[case::seconds(30, "just now")]
    #[case::one_minute(60, "1m ago")]
    #[case::minutes(120, "2m ago")]
    #[case::one_hour(3600, "1h ago")]
    #[case::hours(7200, "2h ago")]
    #[case::one_day(86400, "1d ago")]
    #[case::days(172800, "2d ago")]
    fn test_format_relative_time(#[case] seconds_ago: i64, #[case] expected: &str) {
        let dt = Utc::now() - Duration::seconds(seconds_ago);
        assert_eq!(format_relative_time(dt), expected);
    }

    #[rstest]
    #[case::short("hello", 10, "hello")]
    #[case::exact("hello", 5, "hello")]
    #[case::truncate("hello world", 8, "hello...")]
    #[case::truncate_short("hello", 4, "h...")]
    fn test_truncate(#[case] input: &str, #[case] max_len: usize, #[case] expected: &str) {
        assert_eq!(truncate(input, max_len), expected);
    }

    #[test]
    fn test_status_display() {
        assert_eq!(SessionStatus::Running.display_symbol(), "●");
        assert_eq!(SessionStatus::WaitingInput.display_symbol(), "◐");
        assert_eq!(SessionStatus::Stopped.display_symbol(), "○");

        assert_eq!(SessionStatus::Running.display_name(), "running");
        assert_eq!(SessionStatus::WaitingInput.display_name(), "waiting");
        assert_eq!(SessionStatus::Stopped.display_name(), "stopped");
    }
}
