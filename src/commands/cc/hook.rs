use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;

use anyhow::Result;
use chrono::Utc;
use clap::Args;
use lazy_regex::regex_replace_all;

use super::claude_sessions;
use super::error::CcError;
use super::store;
use super::tty;
use super::types::{HookEvent, HookInput, Session, SessionStatus, TmuxInfo};
use crate::infra::tmux;
use crate::shared::cache;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct HookArgs {
    /// Hook event name (e.g., user-prompt-submit, stop, notification)
    pub event: String,
}

/// Runs the hook command.
/// Reads JSON input from stdin and updates the session state.
pub fn run(args: &HookArgs) -> Result<()> {
    // Parse event type
    let event = HookEvent::from_str(&args.event)?;

    // Read JSON from stdin
    let input = read_stdin_json()?;

    // Handle session end by deleting the session file
    if event == HookEvent::SessionEnd {
        return store::delete_session(&input.session_id);
    }

    // Get TTY from ancestor processes
    let tty = tty::get_tty_from_ancestors();

    // Get tmux info if TTY is available
    let tmux_info = tty.as_ref().and_then(|t| {
        tmux::get_pane_info_by_tty(t).map(|info| TmuxInfo {
            session_name: info.session_name,
            window_name: info.window_name,
            window_index: info.window_index,
            pane_id: info.pane_id,
        })
    });

    // Determine status based on event
    let status = determine_status(event, &input);

    // Load existing session or create new one
    let now = Utc::now();
    let mut session = store::load_session(&input.session_id)?.unwrap_or_else(|| Session {
        session_id: input.session_id.clone(),
        cwd: input.cwd.clone(),
        transcript_path: input.transcript_path.clone(),
        tty: tty.clone(),
        tmux_info: tmux_info.clone(),
        status,
        created_at: now,
        updated_at: now,
        last_message: None,
        current_tool: None,
    });

    // Update session fields
    session.cwd.clone_from(&input.cwd);
    session.updated_at = now;
    session.status = status;

    // Update TTY and tmux info if available
    if tty.is_some() {
        session.tty = tty;
    }
    if tmux_info.is_some() {
        session.tmux_info = tmux_info;
    }
    if input.transcript_path.is_some() {
        session.transcript_path.clone_from(&input.transcript_path);
    }

    // Update last_message from Claude Code's transcript
    session.last_message =
        claude_sessions::get_last_assistant_message(&session.cwd, &session.session_id);

    // Update current_tool based on event type
    session.current_tool = match event {
        HookEvent::PreToolUse => format_current_tool(&input),
        HookEvent::PostToolUse | HookEvent::Stop => None,
        _ => session.current_tool, // Keep existing value for other events
    };

    // Save updated session
    store::save_session(&session)?;

    Ok(())
}

/// Reads and parses JSON from stdin.
fn read_stdin_json() -> Result<HookInput> {
    let mut json_str = String::new();
    io::stdin().lock().read_to_string(&mut json_str)?;

    if json_str.is_empty() {
        return Err(CcError::NoStdinInput.into());
    }

    match serde_json::from_str(&json_str) {
        Ok(input) => Ok(input),
        Err(e) => {
            let log_path = write_error_log(&json_str);
            Err(CcError::JsonParseError {
                source: e,
                log_path,
            }
            .into())
        }
    }
}

/// Writes the raw stdin content to a log file for debugging.
/// Returns the path to the log file if successful, None otherwise.
fn write_error_log(content: &str) -> Option<PathBuf> {
    let logs_dir = logs_dir()?;
    write_error_log_to_dir(content, &logs_dir)
}

/// Writes the raw stdin content to a log file in the specified directory.
/// Returns the path to the log file if successful, None otherwise.
fn write_error_log_to_dir(content: &str, logs_dir: &PathBuf) -> Option<PathBuf> {
    let timestamp = Utc::now().format("%Y%m%d_%H%M%S_%3f");
    let filename = format!("hook_error_{timestamp}.log");
    let log_path = logs_dir.join(&filename);

    if let Err(e) = fs::create_dir_all(logs_dir) {
        eprintln!("Warning: Failed to create logs directory: {e}");
        return None;
    }

    match write_file_with_permissions(&log_path, content) {
        Ok(()) => {
            eprintln!("Raw stdin content saved to: {}", log_path.display());
            Some(log_path)
        }
        Err(e) => {
            eprintln!("Warning: Failed to write error log: {e}");
            None
        }
    }
}

/// Writes content to a file with restrictive permissions (0600 on Unix).
fn write_file_with_permissions(path: &PathBuf, content: &str) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(content.as_bytes())?;
    }

    #[cfg(not(unix))]
    {
        let mut file = fs::File::create(path)?;
        file.write_all(content.as_bytes())?;
    }

    Ok(())
}

/// Returns the directory for storing error logs.
///
/// Path: ~/Library/Caches/armyknife/cc/logs/ (macOS) or ~/.cache/armyknife/cc/logs/ (Linux)
///
/// Note: Ideally logs should go to XDG_STATE_HOME (~/.local/state/), but the `dirs` crate
/// doesn't support state_dir() on macOS. Using cache dir for cross-platform consistency.
fn logs_dir() -> Option<PathBuf> {
    cache::base_dir().map(|d| d.join("cc").join("logs"))
}

/// Formats the current tool display string from hook input.
/// Returns format like "Bash(cargo test)" or "Read(src/main.rs)" or just "Task".
///
/// Strips ANSI escape sequences to prevent terminal injection.
fn format_current_tool(input: &HookInput) -> Option<String> {
    let tool_name = input.tool_name.as_deref()?;

    let detail = input.tool_input.as_ref().and_then(|ti| {
        // Try command (Bash), then file_path (Read/Write/Edit), then pattern (Grep/Glob)
        ti.command
            .as_deref()
            .or(ti.file_path.as_deref())
            .or(ti.pattern.as_deref())
    });

    let result = match detail {
        Some(d) => format!("{}({})", tool_name, d),
        None => tool_name.to_string(),
    };

    // Strip ANSI escape sequences
    Some(regex_replace_all!(r"\x1b\[[0-9;]*[A-Za-z]", &result, |_| "").to_string())
}

/// Determines the session status based on the event and input.
/// Note: SessionEnd is handled separately in run() before this function is called.
fn determine_status(event: HookEvent, input: &HookInput) -> SessionStatus {
    match event {
        HookEvent::Stop => SessionStatus::Stopped,
        HookEvent::Notification => match input.notification_type.as_deref() {
            Some("permission_prompt") => SessionStatus::WaitingInput,
            Some("idle_prompt") => SessionStatus::Stopped,
            _ => SessionStatus::Running,
        },
        HookEvent::UserPromptSubmit
        | HookEvent::PreToolUse
        | HookEvent::PostToolUse
        | HookEvent::SessionEnd => SessionStatus::Running,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    fn create_test_input(notification_type: Option<&str>) -> HookInput {
        let json = match notification_type {
            Some(t) => format!(
                r#"{{"session_id":"test-123","cwd":"/tmp/test","notification_type":"{}"}}"#,
                t
            ),
            None => r#"{"session_id":"test-123","cwd":"/tmp/test"}"#.to_string(),
        };
        serde_json::from_str(&json).expect("valid JSON")
    }

    #[rstest]
    #[case::user_prompt_submit(HookEvent::UserPromptSubmit, None, SessionStatus::Running)]
    #[case::pre_tool_use(HookEvent::PreToolUse, None, SessionStatus::Running)]
    #[case::post_tool_use(HookEvent::PostToolUse, None, SessionStatus::Running)]
    #[case::stop(HookEvent::Stop, None, SessionStatus::Stopped)]
    #[case::notification_generic(HookEvent::Notification, Some("info"), SessionStatus::Running)]
    #[case::notification_permission(
        HookEvent::Notification,
        Some("permission_prompt"),
        SessionStatus::WaitingInput
    )]
    #[case::notification_idle(HookEvent::Notification, Some("idle_prompt"), SessionStatus::Stopped)]
    fn test_determine_status(
        #[case] event: HookEvent,
        #[case] notification_type: Option<&str>,
        #[case] expected: SessionStatus,
    ) {
        let input = create_test_input(notification_type);
        let result = determine_status(event, &input);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_hook_event_parsing() {
        assert_eq!(
            HookEvent::from_str("user-prompt-submit").expect("valid event"),
            HookEvent::UserPromptSubmit
        );
        assert_eq!(
            HookEvent::from_str("pre-tool-use").expect("valid event"),
            HookEvent::PreToolUse
        );
        assert_eq!(
            HookEvent::from_str("post-tool-use").expect("valid event"),
            HookEvent::PostToolUse
        );
        assert_eq!(
            HookEvent::from_str("notification").expect("valid event"),
            HookEvent::Notification
        );
        assert_eq!(
            HookEvent::from_str("stop").expect("valid event"),
            HookEvent::Stop
        );
        assert_eq!(
            HookEvent::from_str("session-end").expect("valid event"),
            HookEvent::SessionEnd
        );

        assert!(HookEvent::from_str("unknown").is_err());
    }

    mod write_error_log_tests {
        use super::*;
        use tempfile::TempDir;

        #[test]
        fn creates_log_file_with_content() {
            let temp_dir = TempDir::new().expect("temp dir creation should succeed");
            let logs_dir = temp_dir.path().to_path_buf();

            let content = r#"{"invalid": json"#;
            let log_path = write_error_log_to_dir(content, &logs_dir).expect("should succeed");

            assert!(log_path.exists(), "log file should be created");
            let written = fs::read_to_string(&log_path).expect("should read log file");
            assert_eq!(written, content);
        }

        #[test]
        fn log_filename_contains_timestamp() {
            let temp_dir = TempDir::new().expect("temp dir creation should succeed");
            let logs_dir = temp_dir.path().to_path_buf();

            let log_path =
                write_error_log_to_dir("test content", &logs_dir).expect("should succeed");
            let filename = log_path
                .file_name()
                .expect("should have filename")
                .to_string_lossy();

            assert!(
                filename.starts_with("hook_error_"),
                "filename should start with hook_error_"
            );
            assert!(filename.ends_with(".log"), "filename should end with .log");
        }

        #[test]
        fn logs_dir_uses_cache_directory() {
            let logs = logs_dir().expect("should have cache directory");
            assert!(
                logs.ends_with("cc/logs"),
                "logs dir should end with cc/logs, got: {logs:?}"
            );
        }

        #[cfg(unix)]
        #[test]
        fn log_file_has_restrictive_permissions() {
            use std::os::unix::fs::PermissionsExt;

            let temp_dir = TempDir::new().expect("temp dir creation should succeed");
            let logs_dir = temp_dir.path().to_path_buf();

            let log_path =
                write_error_log_to_dir("test content", &logs_dir).expect("should succeed");
            let metadata = fs::metadata(&log_path).expect("should get metadata");
            let mode = metadata.permissions().mode() & 0o777;

            assert_eq!(mode, 0o600, "log file should have 0600 permissions");
        }
    }
}
