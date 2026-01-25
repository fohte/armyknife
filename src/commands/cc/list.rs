use std::io::{self, Write};

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

    let mut stdout = io::stdout().lock();
    render_sessions(&mut stdout, &sessions, Utc::now())?;

    Ok(())
}

/// Renders sessions to the given writer.
/// Separated from run() to enable testing.
fn render_sessions<W: Write>(
    writer: &mut W,
    sessions: &[Session],
    now: chrono::DateTime<Utc>,
) -> Result<()> {
    if sessions.is_empty() {
        writeln!(writer, "No active Claude Code sessions.")?;
        return Ok(());
    }

    // Print header
    writeln!(
        writer,
        "{:<24} {:<20} {:<10} UPDATED",
        "SESSION", "WINDOW", "STATUS"
    )?;

    // Print each session
    for session in sessions {
        render_session_row(writer, session, now)?;
    }

    Ok(())
}

/// Renders a single session row to the given writer.
fn render_session_row<W: Write>(
    writer: &mut W,
    session: &Session,
    now: chrono::DateTime<Utc>,
) -> Result<()> {
    let session_name = get_session_display_name(session);
    let window_name = get_window_display_name(session);
    let status_display = format_status(session.status);
    let updated_display = format_relative_time(session.updated_at, now);

    writeln!(
        writer,
        "{:<24} {:<20} {} {} {}",
        truncate(&session_name, 24),
        truncate(&window_name, 20),
        session.status.display_symbol(),
        status_display,
        updated_display
    )?;

    Ok(())
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
/// Padding is applied inside color codes to ensure correct column alignment.
fn format_status(status: SessionStatus) -> String {
    let name = status.display_name();

    // Apply padding inside ANSI codes to avoid column misalignment
    match status {
        SessionStatus::Running => format!("\x1b[32m{name:<8}\x1b[0m"), // Green
        SessionStatus::WaitingInput => format!("\x1b[33m{name:<8}\x1b[0m"), // Yellow
        SessionStatus::Stopped => format!("\x1b[90m{name:<8}\x1b[0m"), // Gray
    }
}

/// Formats a datetime as a relative time string from a given reference time.
fn format_relative_time(dt: chrono::DateTime<Utc>, now: chrono::DateTime<Utc>) -> String {
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

/// Truncates a string to the specified length (character-based, not byte-based).
fn truncate(s: &str, max_len: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_len {
        s.to_string()
    } else if max_len < 3 {
        // Too short for ellipsis, just truncate
        s.chars().take(max_len).collect()
    } else {
        let truncated: String = s.chars().take(max_len - 3).collect();
        format!("{truncated}...")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::types::TmuxInfo;
    use chrono::Duration;
    use indoc::indoc;
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
        let now = Utc::now();
        let dt = now - Duration::seconds(seconds_ago);
        assert_eq!(format_relative_time(dt, now), expected);
    }

    #[rstest]
    #[case::short("hello", 10, "hello")]
    #[case::exact("hello", 5, "hello")]
    #[case::truncate("hello world", 8, "hello...")]
    #[case::truncate_short("hello", 4, "h...")]
    #[case::max_len_3("hello", 3, "...")]
    #[case::max_len_2("hello", 2, "he")]
    #[case::max_len_1("hello", 1, "h")]
    #[case::max_len_0("hello", 0, "")]
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

    #[test]
    fn test_render_sessions_empty() {
        let mut output = Vec::new();
        render_sessions(&mut output, &[], Utc::now()).expect("render should succeed");

        let result = String::from_utf8(output).expect("valid utf8");
        assert_eq!(result, "No active Claude Code sessions.\n");
    }

    #[test]
    fn test_render_sessions_single_without_tmux() {
        let now = Utc::now();
        let mut session = create_test_session();
        session.updated_at = now;

        let mut output = Vec::new();
        render_sessions(&mut output, &[session], now).expect("render should succeed");

        let result = String::from_utf8(output).expect("valid utf8");
        assert_eq!(
            result,
            indoc! {"
                SESSION                  WINDOW               STATUS     UPDATED
                myproject                -                    ● \x1b[32mrunning \x1b[0m just now
            "}
        );
    }

    #[test]
    fn test_render_sessions_single_with_tmux() {
        let now = Utc::now();
        let mut session = create_test_session();
        session.updated_at = now;
        session.tmux_info = Some(TmuxInfo {
            session_name: "dev".to_string(),
            window_name: "editor".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        });

        let mut output = Vec::new();
        render_sessions(&mut output, &[session], now).expect("render should succeed");

        let result = String::from_utf8(output).expect("valid utf8");
        assert_eq!(
            result,
            indoc! {"
                SESSION                  WINDOW               STATUS     UPDATED
                dev                      editor               ● \x1b[32mrunning \x1b[0m just now
            "}
        );
    }

    #[test]
    fn test_render_sessions_multiple_statuses() {
        let now = Utc::now();
        let sessions = vec![
            Session {
                session_id: "s1".to_string(),
                cwd: PathBuf::from("/project/running"),
                transcript_path: None,
                tty: None,
                tmux_info: None,
                status: SessionStatus::Running,
                created_at: now,
                updated_at: now,
                last_message: None,
            },
            Session {
                session_id: "s2".to_string(),
                cwd: PathBuf::from("/project/waiting"),
                transcript_path: None,
                tty: None,
                tmux_info: None,
                status: SessionStatus::WaitingInput,
                created_at: now,
                updated_at: now,
                last_message: None,
            },
            Session {
                session_id: "s3".to_string(),
                cwd: PathBuf::from("/project/stopped"),
                transcript_path: None,
                tty: None,
                tmux_info: None,
                status: SessionStatus::Stopped,
                created_at: now,
                updated_at: now,
                last_message: None,
            },
        ];

        let mut output = Vec::new();
        render_sessions(&mut output, &sessions, now).expect("render should succeed");

        let result = String::from_utf8(output).expect("valid utf8");
        assert_eq!(
            result,
            indoc! {"
                SESSION                  WINDOW               STATUS     UPDATED
                running                  -                    ● \x1b[32mrunning \x1b[0m just now
                waiting                  -                    ◐ \x1b[33mwaiting \x1b[0m just now
                stopped                  -                    ○ \x1b[90mstopped \x1b[0m just now
            "}
        );
    }

    #[test]
    fn test_render_sessions_truncates_long_names() {
        let now = Utc::now();
        let mut session = create_test_session();
        session.updated_at = now;
        session.tmux_info = Some(TmuxInfo {
            session_name: "this-is-a-very-long-session-name-that-exceeds-limit".to_string(),
            window_name: "also-a-very-long-window-name".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        });

        let mut output = Vec::new();
        render_sessions(&mut output, &[session], now).expect("render should succeed");

        let result = String::from_utf8(output).expect("valid utf8");
        assert_eq!(
            result,
            indoc! {"
                SESSION                  WINDOW               STATUS     UPDATED
                this-is-a-very-long-s... also-a-very-long-... ● \x1b[32mrunning \x1b[0m just now
            "}
        );
    }

    #[test]
    fn test_render_sessions_relative_time() {
        let now = Utc::now();
        let mut session = create_test_session();
        session.updated_at = now - Duration::hours(2);

        let mut output = Vec::new();
        render_sessions(&mut output, &[session], now).expect("render should succeed");

        let result = String::from_utf8(output).expect("valid utf8");
        assert_eq!(
            result,
            indoc! {"
                SESSION                  WINDOW               STATUS     UPDATED
                myproject                -                    ● \x1b[32mrunning \x1b[0m 2h ago
            "}
        );
    }
}
