use std::env;
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
use crate::infra::notification::{Notification, NotificationAction};
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

    // Send notification if applicable (errors are logged but don't fail the hook)
    if should_notify(event, &input) {
        send_notification(event, &input, &session);
    }

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

/// Checks if notifications are enabled via environment variable.
fn is_notification_enabled() -> bool {
    match env::var("ARMYKNIFE_CC_NOTIFY") {
        Ok(val) => !matches!(val.to_lowercase().as_str(), "0" | "false"),
        Err(_) => true, // enabled by default
    }
}

/// Determines if a notification should be sent for the given event.
fn should_notify(event: HookEvent, input: &HookInput) -> bool {
    is_notification_enabled() && is_notifiable_event(event, input)
}

/// Checks if the event type and input warrant a notification.
fn is_notifiable_event(event: HookEvent, input: &HookInput) -> bool {
    match event {
        HookEvent::Stop => true,
        HookEvent::Notification => {
            matches!(
                input.notification_type.as_deref(),
                Some("permission_prompt")
            )
        }
        _ => false,
    }
}

/// Sends a notification for the given event.
/// Errors are printed to stderr but don't fail the hook.
fn send_notification(event: HookEvent, input: &HookInput, session: &Session) {
    let notification = build_notification(event, input, session);

    // Print notification errors to stderr without failing the hook
    if let Err(e) = crate::infra::notification::send(&notification) {
        eprintln!("[armyknife] warning: failed to send notification: {e}");
    }
}

/// Truncates a string to the specified maximum length.
/// If truncated, appends "..." to indicate truncation.
fn truncate_string(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len.saturating_sub(3)).collect();
        format!("{}...", truncated)
    }
}

/// Builds a notification for the given event.
fn build_notification(event: HookEvent, input: &HookInput, session: &Session) -> Notification {
    // Title: "Claude Code - Stopped" or "Claude Code - Waiting"
    let status_label = match session.status {
        SessionStatus::WaitingInput => "Waiting",
        SessionStatus::Stopped => "Stopped",
        SessionStatus::Running => "Running",
    };
    let title = format!("Claude Code - {}", status_label);

    // Subtitle: "session:window | タイトル" format
    // Limit to ~50 characters
    let subtitle = build_subtitle(session);

    // Message: use last_message if available, otherwise fall back to event-based message
    let message = session
        .last_message
        .as_ref()
        .map(|m| truncate_string(m, 100))
        .unwrap_or_else(|| match event {
            HookEvent::Stop => "Session stopped".to_string(),
            HookEvent::Notification => input
                .message
                .clone()
                .unwrap_or_else(|| "Permission required".to_string()),
            _ => "Notification".to_string(),
        });

    let mut notification = Notification::new(&title, message).with_sound("Glass");

    if let Some(subtitle) = subtitle {
        notification = notification.with_subtitle(subtitle);
    }

    // Add click action to focus tmux pane if available
    // Skip action if pane_id cannot be safely quoted (e.g., contains null bytes)
    if let Some(tmux_info) = &session.tmux_info
        && let Ok(escaped_pane_id) = shlex::try_quote(&tmux_info.pane_id)
    {
        // Use tmux switch-client with the first available client
        let command = format!(
            r#"client_name=$(tmux list-clients -F '#{{client_name}}' | head -n1); tmux switch-client -c "$client_name" -t {}; open -a WezTerm"#,
            escaped_pane_id
        );
        notification = notification.with_action(NotificationAction::new(command));
    }

    notification
}

/// Builds the subtitle for a notification.
/// Format: "session:window | タイトル" or just "session:window" if no title.
fn build_subtitle(session: &Session) -> Option<String> {
    let tmux_info = session.tmux_info.as_ref()?;
    let tmux_part = format!("{}:{}", tmux_info.session_name, tmux_info.window_name);

    // Get session title from Claude Code's metadata
    let session_title = claude_sessions::get_session_title(&session.cwd, &session.session_id);

    // Build full subtitle first, then truncate once to avoid double-truncation issues
    let subtitle = match session_title {
        Some(title) if !title.is_empty() => format!("{} | {}", tmux_part, title),
        _ => tmux_part,
    };

    Some(truncate_string(&subtitle, 50))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    fn create_test_input(notification_type: Option<&str>) -> HookInput {
        create_test_input_with_message(notification_type, None)
    }

    fn create_test_input_with_message(
        notification_type: Option<&str>,
        message: Option<&str>,
    ) -> HookInput {
        let mut json_parts = vec![
            r#""session_id":"test-123""#.to_string(),
            r#""cwd":"/tmp/test""#.to_string(),
        ];
        if let Some(t) = notification_type {
            json_parts.push(format!(r#""notification_type":"{}""#, t));
        }
        if let Some(m) = message {
            json_parts.push(format!(r#""message":"{}""#, m));
        }
        let json = format!("{{{}}}", json_parts.join(","));
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

    #[rstest]
    #[case::stop_always_notifies(HookEvent::Stop, None, true)]
    #[case::permission_prompt_notifies(HookEvent::Notification, Some("permission_prompt"), true)]
    #[case::idle_prompt_no_notification(HookEvent::Notification, Some("idle_prompt"), false)]
    #[case::generic_notification_no_notify(HookEvent::Notification, Some("info"), false)]
    #[case::user_prompt_no_notification(HookEvent::UserPromptSubmit, None, false)]
    #[case::pre_tool_no_notification(HookEvent::PreToolUse, None, false)]
    #[case::post_tool_no_notification(HookEvent::PostToolUse, None, false)]
    fn test_is_notifiable_event(
        #[case] event: HookEvent,
        #[case] notification_type: Option<&str>,
        #[case] expected: bool,
    ) {
        let input = create_test_input(notification_type);
        let result = is_notifiable_event(event, &input);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_build_notification_stop_event() {
        let input = create_test_input(None);
        let mut session = create_test_session(None);
        session.status = SessionStatus::Stopped;
        let notification = build_notification(HookEvent::Stop, &input, &session);

        // Title includes status
        assert_eq!(notification.title(), "Claude Code - Stopped");
        // Message falls back to "Session stopped" when no last_message
        assert_eq!(notification.message(), "Session stopped");
        assert_eq!(notification.sound(), Some("Glass"));
        // No subtitle without tmux_info
        assert!(notification.subtitle().is_none());
        assert!(notification.action().is_none());
    }

    #[test]
    fn test_build_notification_permission_with_message() {
        let input = create_test_input_with_message(Some("permission_prompt"), Some("Allow edit?"));
        let mut session = create_test_session(None);
        session.status = SessionStatus::WaitingInput;
        let notification = build_notification(HookEvent::Notification, &input, &session);

        // Title shows Waiting status
        assert_eq!(notification.title(), "Claude Code - Waiting");
        // Message falls back to input.message when no last_message
        assert_eq!(notification.message(), "Allow edit?");
    }

    #[test]
    fn test_build_notification_permission_without_message() {
        let input = create_test_input(Some("permission_prompt"));
        let session = create_test_session(None);
        let notification = build_notification(HookEvent::Notification, &input, &session);

        assert_eq!(notification.message(), "Permission required");
    }

    #[test]
    fn test_build_notification_with_last_message() {
        let input = create_test_input(None);
        let mut session = create_test_session(None);
        session.status = SessionStatus::Stopped;
        session.last_message = Some("I've updated the code as requested.".to_string());
        let notification = build_notification(HookEvent::Stop, &input, &session);

        // Message uses last_message when available
        assert_eq!(
            notification.message(),
            "I've updated the code as requested."
        );
    }

    #[test]
    fn test_build_notification_with_tmux_info() {
        let input = create_test_input(None);
        let mut session = create_test_session(Some(TmuxInfo {
            session_name: "main".to_string(),
            window_name: "dev".to_string(),
            window_index: 1,
            pane_id: "%123".to_string(),
        }));
        session.status = SessionStatus::Stopped;
        let notification = build_notification(HookEvent::Stop, &input, &session);

        // Subtitle should contain session:window (no title since we can't mock Claude sessions)
        assert_eq!(notification.subtitle(), Some("main:dev"));

        // Action should switch to the correct pane
        assert!(notification.action().is_some());
        let action = notification.action().expect("action present");
        assert!(action.command().contains("tmux switch-client"));
        assert!(action.command().contains("%123"));
        assert!(action.command().contains("list-clients"));
    }

    #[test]
    fn test_truncate_string() {
        // String within limit
        assert_eq!(truncate_string("hello", 10), "hello");

        // String at exact limit
        assert_eq!(truncate_string("hello", 5), "hello");

        // String exceeds limit
        assert_eq!(truncate_string("hello world", 8), "hello...");

        // Very short limit
        assert_eq!(truncate_string("hello world", 5), "he...");
    }

    fn create_test_session(tmux_info: Option<TmuxInfo>) -> Session {
        Session {
            session_id: "test-123".to_string(),
            cwd: "/tmp/test".into(),
            transcript_path: None,
            tty: None,
            tmux_info,
            status: SessionStatus::Running,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
        }
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
