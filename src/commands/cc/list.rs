use std::io::{self, Write};

use anyhow::Result;
use chrono::Utc;
use clap::Args;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::claude_sessions;
use super::store;
use super::types::{Session, SessionStatus};

/// Column widths for fixed-width columns
const SESSION_WIDTH: usize = 16;
const WINDOW_WIDTH: usize = 12;
/// STATUS column: symbol (1-2) + space (1) + name (8) + space (1) = 11
const STATUS_WIDTH: usize = 11;
/// UPDATED column: "just now" (8) or "XXXd ago" (8) = 8
const UPDATED_WIDTH: usize = 8;
/// Minimum width for TITLE column
const MIN_TITLE_WIDTH: usize = 20;
/// Spaces between columns
const COLUMN_SPACES: usize = 5;

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
    let term_width = get_terminal_width();
    render_sessions(&mut stdout, &sessions, Utc::now(), term_width)?;

    Ok(())
}

/// Gets the terminal width, defaulting to 80 if unavailable.
fn get_terminal_width() -> usize {
    crossterm::terminal::size()
        .map(|(w, _)| w as usize)
        .unwrap_or(80)
}

/// Calculates the title column width based on terminal width.
fn calculate_title_width(term_width: usize) -> usize {
    let fixed_width = SESSION_WIDTH + WINDOW_WIDTH + STATUS_WIDTH + UPDATED_WIDTH + COLUMN_SPACES;
    if term_width > fixed_width + MIN_TITLE_WIDTH {
        term_width - fixed_width
    } else {
        MIN_TITLE_WIDTH
    }
}

/// Renders sessions to the given writer.
/// Separated from run() to enable testing.
fn render_sessions<W: Write>(
    writer: &mut W,
    sessions: &[Session],
    now: chrono::DateTime<Utc>,
    term_width: usize,
) -> Result<()> {
    if sessions.is_empty() {
        writeln!(writer, "No active Claude Code sessions.")?;
        return Ok(());
    }

    let title_width = calculate_title_width(term_width);

    // Print header
    writeln!(
        writer,
        "{} {} {} {:<10} UPDATED",
        pad_or_truncate("TITLE", title_width),
        pad_or_truncate("SESSION", SESSION_WIDTH),
        pad_or_truncate("WINDOW", WINDOW_WIDTH),
        "STATUS"
    )?;

    // Print each session
    for session in sessions {
        render_session_row(writer, session, now, title_width)?;
    }

    Ok(())
}

/// Renders a single session row to the given writer.
fn render_session_row<W: Write>(
    writer: &mut W,
    session: &Session,
    now: chrono::DateTime<Utc>,
    title_width: usize,
) -> Result<()> {
    let title = get_title_display_name(session);
    let session_name = get_session_display_name(session);
    let window_name = get_window_display_name(session);
    let status_display = format_status(session.status);
    let updated_display = format_relative_time(session.updated_at, now);

    writeln!(
        writer,
        "{} {} {} {} {} {}",
        pad_or_truncate(&title, title_width),
        pad_or_truncate(&session_name, SESSION_WIDTH),
        pad_or_truncate(&window_name, WINDOW_WIDTH),
        session.status.display_symbol(),
        status_display,
        updated_display
    )?;

    Ok(())
}

/// Gets the title display name for a session.
/// Fetches from Claude Code's sessions-index.json, returns "-" if not found.
fn get_title_display_name(session: &Session) -> String {
    claude_sessions::get_session_title(&session.cwd, &session.session_id)
        .unwrap_or_else(|| "-".to_string())
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

/// Pads or truncates a string to exactly the specified display width.
/// Uses unicode display width for proper alignment with CJK characters.
fn pad_or_truncate(s: &str, width: usize) -> String {
    let display_width = s.width();

    if display_width <= width {
        // Pad with spaces to reach target width
        let padding = width - display_width;
        format!("{}{}", s, " ".repeat(padding))
    } else if width < 3 {
        // Too short for ellipsis, just truncate
        truncate_to_width(s, width)
    } else {
        // Truncate and add ellipsis
        let truncated = truncate_to_width(s, width - 3);
        let truncated_width = truncated.width();
        // Use saturating_sub to avoid underflow when CJK chars cause width mismatch
        let padding = width.saturating_sub(truncated_width).saturating_sub(3);
        format!("{}...{}", truncated, " ".repeat(padding))
    }
}

/// Truncates a string to fit within the specified display width.
fn truncate_to_width(s: &str, max_width: usize) -> String {
    let mut result = String::new();
    let mut current_width = 0;

    for c in s.chars() {
        let char_width = c.width().unwrap_or(0);
        if current_width + char_width > max_width {
            break;
        }
        result.push(c);
        current_width += char_width;
    }

    result
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
    #[case::short("hello", 10, "hello     ")]
    #[case::exact("hello", 5, "hello")]
    #[case::truncate("hello world", 8, "hello...")]
    #[case::truncate_short("hello", 4, "h...")]
    #[case::max_len_3("hello", 3, "...")]
    #[case::max_len_2("hello", 2, "he")]
    #[case::max_len_1("hello", 1, "h")]
    #[case::max_len_0("hello", 0, "")]
    #[case::cjk_short("日本語", 10, "日本語    ")]
    #[case::cjk_exact("日本語", 6, "日本語")]
    #[case::cjk_truncate("日本語テスト", 8, "日本... ")]
    fn test_pad_or_truncate(#[case] input: &str, #[case] width: usize, #[case] expected: &str) {
        assert_eq!(pad_or_truncate(input, width), expected);
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

    /// Test terminal width that results in TITLE width of 30
    const TEST_TERM_WIDTH: usize = 82;

    #[test]
    fn test_render_sessions_empty() {
        let mut output = Vec::new();
        render_sessions(&mut output, &[], Utc::now(), TEST_TERM_WIDTH)
            .expect("render should succeed");

        let result = String::from_utf8(output).expect("valid utf8");
        assert_eq!(result, "No active Claude Code sessions.\n");
    }

    #[test]
    fn test_render_sessions_single_without_tmux() {
        let now = Utc::now();
        let mut session = create_test_session();
        session.updated_at = now;

        let mut output = Vec::new();
        render_sessions(&mut output, &[session], now, TEST_TERM_WIDTH)
            .expect("render should succeed");

        let result = String::from_utf8(output).expect("valid utf8");
        assert_eq!(
            result,
            indoc! {"
                TITLE                          SESSION          WINDOW       STATUS     UPDATED
                -                              myproject        -            ● \x1b[32mrunning \x1b[0m just now
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
        render_sessions(&mut output, &[session], now, TEST_TERM_WIDTH)
            .expect("render should succeed");

        let result = String::from_utf8(output).expect("valid utf8");
        assert_eq!(
            result,
            indoc! {"
                TITLE                          SESSION          WINDOW       STATUS     UPDATED
                -                              dev              editor       ● \x1b[32mrunning \x1b[0m just now
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
        render_sessions(&mut output, &sessions, now, TEST_TERM_WIDTH)
            .expect("render should succeed");

        let result = String::from_utf8(output).expect("valid utf8");
        // Title is fetched from Claude Code's sessions-index.json, returns "-" for test paths
        assert_eq!(
            result,
            indoc! {"
                TITLE                          SESSION          WINDOW       STATUS     UPDATED
                -                              running          -            ● \x1b[32mrunning \x1b[0m just now
                -                              waiting          -            ◐ \x1b[33mwaiting \x1b[0m just now
                -                              stopped          -            ○ \x1b[90mstopped \x1b[0m just now
            "}
        );
    }

    #[test]
    fn test_render_sessions_truncates_long_names() {
        let now = Utc::now();
        let mut session = create_test_session();
        session.updated_at = now;
        session.tmux_info = Some(TmuxInfo {
            session_name: "this-is-a-very-long-session-name".to_string(),
            window_name: "also-a-very-long-window-name".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        });

        let mut output = Vec::new();
        render_sessions(&mut output, &[session], now, TEST_TERM_WIDTH)
            .expect("render should succeed");

        let result = String::from_utf8(output).expect("valid utf8");
        // Title returns "-" (no sessions-index.json), session/window names are truncated
        assert_eq!(
            result,
            indoc! {"
                TITLE                          SESSION          WINDOW       STATUS     UPDATED
                -                              this-is-a-ver... also-a-ve... ● \x1b[32mrunning \x1b[0m just now
            "}
        );
    }

    #[test]
    fn test_render_sessions_relative_time() {
        let now = Utc::now();
        let mut session = create_test_session();
        session.updated_at = now - Duration::hours(2);

        let mut output = Vec::new();
        render_sessions(&mut output, &[session], now, TEST_TERM_WIDTH)
            .expect("render should succeed");

        let result = String::from_utf8(output).expect("valid utf8");
        assert_eq!(
            result,
            indoc! {"
                TITLE                          SESSION          WINDOW       STATUS     UPDATED
                -                              myproject        -            ● \x1b[32mrunning \x1b[0m 2h ago
            "}
        );
    }

    #[test]
    fn test_get_title_display_name_without_sessions_index() {
        // When sessions-index.json doesn't exist, returns "-"
        let session = create_test_session();
        assert_eq!(get_title_display_name(&session), "-");
    }
}
