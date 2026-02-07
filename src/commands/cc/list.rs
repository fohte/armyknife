use std::io::{self, Write};

use anyhow::Result;
use chrono::Utc;
use clap::Args;

use super::claude_sessions;
use super::store;
use super::types::{Session, SessionStatus};
use crate::shared::table::{color, pad_or_truncate};

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
pub struct ListArgs {
    /// Output short status for tmux status bar
    #[arg(long)]
    pub tmux: bool,
}

/// Runs the list command.
/// Displays all Claude Code sessions in a formatted table.
pub fn run(args: &ListArgs) -> Result<()> {
    // Clean up stale sessions first
    store::cleanup_stale_sessions()?;

    // Load all sessions
    let sessions = store::list_sessions()?;

    if args.tmux {
        let mut stdout = io::stdout().lock();
        render_tmux_status(&mut stdout, &sessions)?;
        return Ok(());
    }

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

/// Renders a short status string for tmux status bar.
///
/// - If any session is WaitingInput: `#[fg=yellow]◐ N` (N = count of non-running sessions)
/// - If only Stopped sessions exist: `○ N`
/// - If all Running or no sessions: empty string (no output)
fn render_tmux_status<W: Write>(writer: &mut W, sessions: &[Session]) -> Result<()> {
    let (waiting_count, stopped_count) =
        sessions
            .iter()
            .fold((0, 0), |(waiting, stopped), s| match s.status {
                SessionStatus::WaitingInput => (waiting + 1, stopped),
                SessionStatus::Stopped => (waiting, stopped + 1),
                _ => (waiting, stopped),
            });

    let pending_count = waiting_count + stopped_count;
    if pending_count == 0 {
        return Ok(());
    }

    if waiting_count > 0 {
        write!(writer, "#[fg=yellow]\u{25d0} {pending_count}#[default]")?;
    } else {
        write!(writer, "\u{25cb} {pending_count}")?;
    }

    Ok(())
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
    let (col, reset) = match status {
        SessionStatus::Running => (color::GREEN, color::RESET),
        SessionStatus::WaitingInput => (color::YELLOW, color::RESET),
        SessionStatus::Stopped => (color::GRAY, color::RESET),
    };
    format!("{col}{name:<8}{reset}")
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
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
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
                current_tool: None,
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
                current_tool: None,
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
                current_tool: None,
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

    // =========================================================================
    // tmux status output tests
    // =========================================================================

    #[test]
    fn test_render_tmux_status_empty() {
        let mut output = Vec::new();
        render_tmux_status(&mut output, &[]).expect("render should succeed");
        assert_eq!(String::from_utf8(output).expect("valid utf8"), "");
    }

    #[test]
    fn test_render_tmux_status_all_running() {
        let sessions = vec![
            Session {
                status: SessionStatus::Running,
                ..create_test_session()
            },
            Session {
                status: SessionStatus::Running,
                ..create_test_session()
            },
        ];
        let mut output = Vec::new();
        render_tmux_status(&mut output, &sessions).expect("render should succeed");
        assert_eq!(String::from_utf8(output).expect("valid utf8"), "");
    }

    #[test]
    fn test_render_tmux_status_waiting_input() {
        let sessions = vec![
            Session {
                status: SessionStatus::Running,
                ..create_test_session()
            },
            Session {
                status: SessionStatus::WaitingInput,
                ..create_test_session()
            },
            Session {
                status: SessionStatus::WaitingInput,
                ..create_test_session()
            },
        ];
        let mut output = Vec::new();
        render_tmux_status(&mut output, &sessions).expect("render should succeed");
        assert_eq!(
            String::from_utf8(output).expect("valid utf8"),
            "#[fg=yellow]\u{25d0} 2#[default]"
        );
    }

    #[test]
    fn test_render_tmux_status_stopped_only() {
        let sessions = vec![
            Session {
                status: SessionStatus::Running,
                ..create_test_session()
            },
            Session {
                status: SessionStatus::Stopped,
                ..create_test_session()
            },
        ];
        let mut output = Vec::new();
        render_tmux_status(&mut output, &sessions).expect("render should succeed");
        assert_eq!(String::from_utf8(output).expect("valid utf8"), "\u{25cb} 1");
    }

    #[test]
    fn test_render_tmux_status_mixed_waiting_and_stopped() {
        let sessions = vec![
            Session {
                status: SessionStatus::WaitingInput,
                ..create_test_session()
            },
            Session {
                status: SessionStatus::Stopped,
                ..create_test_session()
            },
            Session {
                status: SessionStatus::Running,
                ..create_test_session()
            },
        ];
        let mut output = Vec::new();
        render_tmux_status(&mut output, &sessions).expect("render should succeed");
        // WaitingInput takes priority for the symbol, count includes both
        assert_eq!(
            String::from_utf8(output).expect("valid utf8"),
            "#[fg=yellow]\u{25d0} 2#[default]"
        );
    }

    // =========================================================================
    // Integration tests for terminal width adaptation
    // =========================================================================

    #[test]
    fn test_calculate_title_width_wide_terminal() {
        // Wide terminal (120 chars) should give more space to TITLE
        // fixed_width = 16 + 12 + 11 + 8 + 5 = 52
        // title_width = 120 - 52 = 68
        assert_eq!(calculate_title_width(120), 68);
    }

    #[test]
    fn test_calculate_title_width_narrow_terminal() {
        // Narrow terminal should use minimum TITLE width
        // fixed_width = 52, min_title = 20
        // If term_width <= 72, use minimum
        assert_eq!(calculate_title_width(60), MIN_TITLE_WIDTH);
        assert_eq!(calculate_title_width(72), MIN_TITLE_WIDTH);
    }

    #[test]
    fn test_render_sessions_wide_terminal() {
        // Wide terminal (100 chars) - TITLE width = 100 - 52 = 48
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
        render_sessions(&mut output, &[session], now, 100).expect("render should succeed");

        let result = String::from_utf8(output).expect("valid utf8");
        // TITLE width = 48, SESSION = 16, WINDOW = 12
        assert_eq!(
            result,
            indoc! {"
                TITLE                                            SESSION          WINDOW       STATUS     UPDATED
                -                                                dev              editor       ● \x1b[32mrunning \x1b[0m just now
            "}
        );
    }

    #[test]
    fn test_render_sessions_narrow_terminal() {
        // Narrow terminal (60 chars) - uses MIN_TITLE_WIDTH (20)
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
        render_sessions(&mut output, &[session], now, 60).expect("render should succeed");

        let result = String::from_utf8(output).expect("valid utf8");
        // TITLE width = 20 (minimum)
        assert_eq!(
            result,
            indoc! {"
                TITLE                SESSION          WINDOW       STATUS     UPDATED
                -                    dev              editor       ● \x1b[32mrunning \x1b[0m just now
            "}
        );
    }

    #[test]
    fn test_render_sessions_full_output_with_all_statuses() {
        let now = Utc::now();
        let sessions = vec![
            Session {
                session_id: "s1".to_string(),
                cwd: PathBuf::from("/home/user/webapp"),
                transcript_path: None,
                tty: None,
                tmux_info: Some(TmuxInfo {
                    session_name: "webapp".to_string(),
                    window_name: "dev".to_string(),
                    window_index: 0,
                    pane_id: "%0".to_string(),
                }),
                status: SessionStatus::Running,
                created_at: now,
                updated_at: now,
                last_message: None,
                current_tool: None,
            },
            Session {
                session_id: "s2".to_string(),
                cwd: PathBuf::from("/home/user/api"),
                transcript_path: None,
                tty: None,
                tmux_info: Some(TmuxInfo {
                    session_name: "api".to_string(),
                    window_name: "test".to_string(),
                    window_index: 1,
                    pane_id: "%1".to_string(),
                }),
                status: SessionStatus::WaitingInput,
                created_at: now,
                updated_at: now - Duration::minutes(5),
                last_message: None,
                current_tool: None,
            },
            Session {
                session_id: "s3".to_string(),
                cwd: PathBuf::from("/home/user/docs"),
                transcript_path: None,
                tty: None,
                tmux_info: None,
                status: SessionStatus::Stopped,
                created_at: now,
                updated_at: now - Duration::hours(1),
                last_message: None,
                current_tool: None,
            },
        ];

        let mut output = Vec::new();
        render_sessions(&mut output, &sessions, now, TEST_TERM_WIDTH)
            .expect("render should succeed");

        let result = String::from_utf8(output).expect("valid utf8");
        assert_eq!(
            result,
            indoc! {"
                TITLE                          SESSION          WINDOW       STATUS     UPDATED
                -                              webapp           dev          ● \x1b[32mrunning \x1b[0m just now
                -                              api              test         ◐ \x1b[33mwaiting \x1b[0m 5m ago
                -                              docs             -            ○ \x1b[90mstopped \x1b[0m 1h ago
            "}
        );
    }
}
