use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use clap::Args;
use indoc::formatdoc;
use lazy_regex::regex_replace_all;

use super::claude_sessions;
use super::error::CcError;
use super::generate_label;
use super::store;
use super::types::{HookEvent, HookInput, Session, SessionStatus, TMUX_SESSION_OPTION, TmuxInfo};
use crate::infra::notification::{Notification, NotificationAction};
use crate::infra::tmux;
use crate::shared::cache;
use crate::shared::command::find_command_path;
use crate::shared::config::{self, Config};
use crate::shared::env_var::EnvVars;

/// Delay between retries when waiting for transcript to be updated.
const TRANSCRIPT_RETRY_DELAY: Duration = Duration::from_millis(100);

/// Maximum number of retries when transcript hasn't been updated yet.
const TRANSCRIPT_MAX_RETRIES: u32 = 5;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct HookArgs {
    /// Hook event name (e.g., user-prompt-submit, stop, notification)
    pub event: String,
}

/// Runs the hook command.
/// Reads JSON input from stdin and updates the session state.
pub fn run(args: &HookArgs) -> Result<()> {
    // Skip hooks when called from armyknife's own claude -p invocations
    // to prevent infinite recursion (hook → claude -p → hook → ...).
    if EnvVars::load().skip_hooks {
        return Ok(());
    }

    // Read raw stdin first for debug logging
    let raw_stdin = read_raw_stdin()?;

    // Parse event type
    let event = HookEvent::from_str(&args.event)?;

    // Parse JSON from raw stdin
    let log_level = get_log_level();
    let input = match parse_stdin_json(&raw_stdin) {
        Ok(input) => {
            // Log successful parse only at debug level
            if log_level == LogLevel::Debug {
                write_hook_log(&raw_stdin, &args.event, true, None);
            }
            input
        }
        Err(e) => {
            // Log parse error at error level or higher
            if log_level.should_log_errors() {
                write_hook_log(&raw_stdin, &args.event, false, Some(&e.to_string()));
            }
            return Err(e);
        }
    };

    process_hook_event(event, input)
}

/// Result of processing a hook event.
#[derive(Debug, PartialEq, Eq)]
enum ProcessResult {
    /// Session was created or updated
    SessionSaved,
    /// Session was marked as ended (session-end event)
    SessionEnded,
    /// Event was skipped (e.g., resume session-start)
    Skipped,
}

/// Processes a hook event with the given input.
/// This is the core logic separated from stdin handling for testability.
fn process_hook_event(event: HookEvent, input: HookInput) -> Result<()> {
    let sessions_dir = store::sessions_dir()?;
    process_hook_event_impl(event, input, &sessions_dir).map(|_| ())
}

/// Internal implementation that returns ProcessResult for testing.
/// Accepts sessions_dir as a parameter to allow testing with temporary directories.
fn process_hook_event_impl(
    event: HookEvent,
    input: HookInput,
    sessions_dir: &Path,
) -> Result<ProcessResult> {
    let env = EnvVars::load();

    // Handle session end: mark as ended instead of deleting so that
    // `claude -c` resume can restore label and ancestor chain.
    // Ended sessions are garbage-collected by cleanup_stale_sessions.
    if event == HookEvent::SessionEnd {
        if let Some(pane_info) = tmux::get_pane_info_by_pid(std::process::id()) {
            let _ = tmux::unset_pane_option(&pane_info.pane_id, TMUX_SESSION_OPTION);
        }
        if let Some(mut session) = store::load_session_from(sessions_dir, &input.session_id)? {
            session.status = SessionStatus::Ended;
            session.updated_at = Utc::now();
            store::save_session_to(sessions_dir, &session)?;
        }
        let _ = tmux::refresh_status();
        return Ok(ProcessResult::SessionEnded);
    }

    // Handle session start: skip "startup" events to avoid creating empty sessions.
    // When `claude -c` resumes a session, Claude Code fires two SessionStart hooks:
    // - "startup" with a new (unwanted) session_id
    // - "resume" with the actual session_id being restored
    // By skipping "startup", we only create sessions for "resume" events (restored sessions)
    // or when source is absent (backward compatibility / other cases).
    if event == HookEvent::SessionStart {
        // Export session ID to CLAUDE_ENV_FILE so that subsequent Bash commands
        // (e.g., `a wm new`) can automatically discover the parent session ID.
        // This must run for ALL SessionStart events (including "startup") because
        // CLAUDE_ENV_FILE is only writable during SessionStart hooks.
        export_session_id_to_env_file(&input.session_id);

        // Skip "startup" events before setting pane option to avoid setting wrong session_id
        if input.source.as_deref() == Some("startup") {
            return Ok(ProcessResult::Skipped);
        }

        if let Some(pane_info) = tmux::get_pane_info_by_pid(std::process::id()) {
            // Ignore errors; pane option is nice-to-have, not critical
            let _ =
                tmux::set_pane_option(&pane_info.pane_id, TMUX_SESSION_OPTION, &input.session_id);
        }
    }

    // Get tmux info by finding the pane that contains this process
    let tmux_info = tmux::get_pane_info_by_pid(std::process::id()).map(|info| TmuxInfo {
        session_name: info.session_name,
        window_name: info.window_name,
        window_index: info.window_index,
        pane_id: info.pane_id,
    });

    // Determine status based on event
    let status = determine_status(event, &input);

    // Load existing session or create new one
    let now = Utc::now();
    let mut session =
        store::load_session_from(sessions_dir, &input.session_id)?.unwrap_or_else(|| {
            // Read label and ancestor chain from environment variables (set by `wm new`)
            let ancestor_session_ids = env
                .ancestor_session_ids
                .as_ref()
                .map(|s| s.split(',').map(|id| id.trim().to_string()).collect())
                .unwrap_or_default();

            Session {
                session_id: input.session_id.clone(),
                cwd: input.cwd.clone(),
                transcript_path: input.transcript_path.clone(),
                tty: None,
                tmux_info: tmux_info.clone(),
                status,
                created_at: now,
                updated_at: now,
                last_message: None,
                current_tool: None,
                label: env.session_label.clone(),
                ancestor_session_ids,
            }
        });

    // Update session fields
    session.cwd.clone_from(&input.cwd);
    session.updated_at = now;
    session.status = status;

    // Update tmux info if available
    if tmux_info.is_some() {
        session.tmux_info = tmux_info;
    }
    if input.transcript_path.is_some() {
        session.transcript_path.clone_from(&input.transcript_path);
    }

    // Update last_message from Claude Code's transcript.
    // For Stop events, retry if transcript hasn't been updated yet (race condition with
    // Claude Code's write). For other events, read once without retrying.
    let max_retries = if event == HookEvent::Stop {
        TRANSCRIPT_MAX_RETRIES
    } else {
        0
    };
    session.last_message = get_last_message_with_retry(
        &session.cwd,
        &session.session_id,
        session.last_message.as_deref(),
        TRANSCRIPT_RETRY_DELAY,
        max_retries,
    );

    // Update current_tool based on event type
    session.current_tool = match event {
        HookEvent::PreToolUse => format_current_tool(&input),
        HookEvent::PostToolUse | HookEvent::Stop => None,
        _ => session.current_tool, // Keep existing value for other events
    };

    // Save updated session
    store::save_session_to(sessions_dir, &session)?;

    // Auto-generate label for root sessions on first user prompt.
    // Uses the prompt field from UserPromptSubmit stdin JSON directly, because
    // transcript files (.jsonl) are not yet written when the hook fires.
    // Spawns a background process to avoid blocking the hook.
    // Sets a placeholder label before spawning to prevent duplicate spawns
    // when multiple UserPromptSubmit events arrive before generation completes.
    if event == HookEvent::UserPromptSubmit
        && session.label.is_none()
        && let Some(prompt) = &input.prompt
    {
        session.label = Some("...".to_string());
        store::save_session_to(sessions_dir, &session)?;

        generate_label::spawn_label_generation(sessions_dir, &session.session_id, prompt);
    }

    // Refresh tmux status bar so `#()` commands pick up the state change immediately.
    // Silently ignore errors (e.g., not in tmux, tmux not installed).
    let _ = tmux::refresh_status();

    // Send notification if applicable (errors are logged but don't fail the hook).
    // Use default config if loading fails to avoid config errors blocking notifications.
    let config = config::load_config().unwrap_or_default();
    if should_notify(event, &config) {
        send_notification(event, &input, &session, &config);
    }

    Ok(ProcessResult::SessionSaved)
}

/// Reads raw content from stdin.
fn read_raw_stdin() -> Result<String> {
    let mut content = String::new();
    io::stdin().lock().read_to_string(&mut content)?;
    Ok(content)
}

/// Parses JSON from raw stdin content.
fn parse_stdin_json(raw_stdin: &str) -> Result<HookInput> {
    if raw_stdin.is_empty() {
        return Err(CcError::NoStdinInput.into());
    }

    Ok(serde_json::from_str(raw_stdin)?)
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
/// Path: ~/.cache/armyknife/cc/logs/
///
/// Note: Ideally logs should go to XDG_STATE_HOME (~/.local/state/), but the `dirs` crate
/// doesn't support state_dir() on macOS. Using cache dir for cross-platform consistency.
fn logs_dir() -> Option<PathBuf> {
    cache::base_dir().map(|d| d.join("cc").join("logs"))
}

/// Log level for hook logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogLevel {
    /// No logging
    Off,
    /// Log only errors (default)
    Error,
    /// Log everything including successful operations
    Debug,
}

impl LogLevel {
    /// Parse log level from string value.
    fn from_str(value: Option<&str>) -> Self {
        match value {
            Some("debug") => Self::Debug,
            Some("error") => Self::Error,
            Some("off") => Self::Off,
            // Default to error level for unset or unknown values
            _ => Self::Error,
        }
    }

    /// Returns true if errors should be logged at this level.
    fn should_log_errors(self) -> bool {
        matches!(self, Self::Error | Self::Debug)
    }
}

/// Gets the log level from environment variable.
fn get_log_level() -> LogLevel {
    LogLevel::from_str(EnvVars::load().cc_hook_log.as_deref())
}

/// Writes a hook log with stdin content, event type, and processing result.
fn write_hook_log(stdin_content: &str, event: &str, success: bool, error_message: Option<&str>) {
    if let Some(logs_dir) = logs_dir() {
        let _ = write_hook_log_to_dir(stdin_content, event, success, error_message, &logs_dir);
    }
}

/// Writes a hook log to the specified directory.
fn write_hook_log_to_dir(
    stdin_content: &str,
    event: &str,
    success: bool,
    error_message: Option<&str>,
    logs_dir: &PathBuf,
) -> Option<PathBuf> {
    let timestamp = Utc::now().format("%Y%m%d_%H%M%S_%3f");
    let filename = format!("hook_{timestamp}.log");
    let log_path = logs_dir.join(&filename);

    if let Err(e) = fs::create_dir_all(logs_dir) {
        eprintln!("Warning: Failed to create logs directory: {e}");
        return None;
    }

    let status = if success { "success" } else { "error" };
    let error_section = match error_message {
        Some(msg) => formatdoc! {"


            === Error Message ===
            {msg}"},
        None => String::new(),
    };

    let content = formatdoc! {"
        === Event ===
        {event}

        === Status ===
        {status}{error_section}

        === Raw Stdin ===
        {stdin_content}"};

    match write_file_with_permissions(&log_path, &content) {
        Ok(()) => {
            eprintln!("Hook log saved to: {}", log_path.display());
            Some(log_path)
        }
        Err(e) => {
            eprintln!("Warning: Failed to write hook log: {e}");
            None
        }
    }
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

/// Formats a permission request message showing what tool and action is being requested.
/// Returns format like "Bash: cargo test" or "Edit: src/main.rs" or just "Bash".
fn format_permission_request_message(input: &HookInput) -> Option<String> {
    let tool_name = input.tool_name.as_deref()?;

    let detail = input.tool_input.as_ref().and_then(|ti| {
        // Try command (Bash), then file_path (Read/Write/Edit), then pattern (Grep/Glob)
        ti.command
            .as_deref()
            .or(ti.file_path.as_deref())
            .or(ti.pattern.as_deref())
    });

    let result = match detail {
        Some(d) => {
            let truncated = truncate_string(d, 80);
            format!("{}: {}", tool_name, truncated)
        }
        None => tool_name.to_string(),
    };

    // Strip ANSI escape sequences
    Some(regex_replace_all!(r"\x1b\[[0-9;]*[A-Za-z]", &result, |_| "").to_string())
}

/// Reads the last assistant message from the transcript, retrying if it hasn't changed.
///
/// Claude Code may not have written the latest response to the transcript .jsonl file
/// by the time the stop hook fires. To mitigate this race condition, we compare the
/// newly read message against the previously saved one. If they match (suggesting the
/// transcript hasn't been updated yet), we wait briefly and retry.
///
/// Skips retries when there's no previous message (first hook invocation for this session).
fn get_last_message_with_retry(
    cwd: &Path,
    session_id: &str,
    previous_message: Option<&str>,
    retry_delay: Duration,
    max_retries: u32,
) -> Option<String> {
    get_last_message_with_retry_impl(previous_message, retry_delay, max_retries, || {
        claude_sessions::get_last_assistant_message(cwd, session_id)
    })
}

/// Internal implementation that accepts a closure for testability.
fn get_last_message_with_retry_impl<F>(
    previous_message: Option<&str>,
    retry_delay: Duration,
    max_retries: u32,
    read_message: F,
) -> Option<String>
where
    F: Fn() -> Option<String>,
{
    // No previous message means this is the first read for this session; no retry needed.
    let Some(prev) = previous_message else {
        return read_message();
    };

    for i in 0..=max_retries {
        let current = read_message();

        // If the message has changed, or this is the last attempt, return the result.
        if current.as_deref() != Some(prev) || i == max_retries {
            return current;
        }

        // Message is still unchanged, wait before retrying.
        thread::sleep(retry_delay);
    }

    // Unreachable: the loop always returns.
    unreachable!()
}

/// Exports the session ID to Claude Code's env file so that subsequent Bash commands
/// can access it as `$ARMYKNIFE_SESSION_ID`.
///
/// Claude Code provides `CLAUDE_ENV_FILE` only during SessionStart hooks.
/// Writing `export ARMYKNIFE_SESSION_ID=...` to this file makes the variable
/// available in all subsequent Bash tool executions within the session.
fn export_session_id_to_env_file(session_id: &str) {
    if let Ok(env_file) = env::var("CLAUDE_ENV_FILE") {
        let export_line = format!("export {}=\"{}\"\n", EnvVars::session_id_name(), session_id);
        // Append to preserve variables set by other hooks
        let _ = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(env_file)
            .and_then(|mut f| f.write_all(export_line.as_bytes()));
    }
}

/// Determines the session status based on the event and input.
/// Note: SessionEnd is handled separately in run() before this function is called.
fn determine_status(event: HookEvent, input: &HookInput) -> SessionStatus {
    match event {
        HookEvent::Stop => SessionStatus::Stopped,
        HookEvent::PermissionRequest => SessionStatus::WaitingInput,
        HookEvent::Notification => match input.notification_type.as_deref() {
            Some("permission_prompt") => SessionStatus::WaitingInput,
            Some("idle_prompt") => SessionStatus::Stopped,
            _ => SessionStatus::Running,
        },
        HookEvent::SessionStart
        | HookEvent::UserPromptSubmit
        | HookEvent::PreToolUse
        | HookEvent::PostToolUse
        | HookEvent::SessionEnd => SessionStatus::Running,
    }
}

/// Checks if notifications are enabled via environment variable.
fn is_notification_enabled(config: &Config) -> bool {
    // Environment variable takes precedence over config for backward compatibility
    match EnvVars::load().cc_notify {
        Some(val) => !matches!(val.to_lowercase().as_str(), "0" | "false"),
        None => config.notification.enabled,
    }
}

/// Determines if a notification should be sent for the given event.
fn should_notify(event: HookEvent, config: &Config) -> bool {
    is_notification_enabled(config) && is_notifiable_event(event)
}

/// Checks if the event type warrants a notification.
/// Uses PermissionRequest for permission notifications (has tool details),
/// skips Notification/permission_prompt to avoid duplicates.
fn is_notifiable_event(event: HookEvent) -> bool {
    matches!(event, HookEvent::Stop | HookEvent::PermissionRequest)
}

/// Sends a notification for the given event.
/// Errors are printed to stderr but don't fail the hook.
fn send_notification(event: HookEvent, input: &HookInput, session: &Session, config: &Config) {
    let notification = build_notification(event, input, session, config);

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
fn build_notification(
    event: HookEvent,
    input: &HookInput,
    session: &Session,
    config: &Config,
) -> Notification {
    // Title: "Claude Code - Stopped" or "Claude Code - Waiting"
    let status_label = match session.status {
        SessionStatus::WaitingInput => "Waiting",
        SessionStatus::Stopped => "Stopped",
        SessionStatus::Running => "Running",
        SessionStatus::Ended => "Ended",
    };
    let title = format!("Claude Code - {}", status_label);

    // Subtitle: "session:window | タイトル" format
    // Limit to ~50 characters
    let subtitle = build_subtitle(session);

    // Message: for permission requests, show tool details (e.g., "Bash: cargo test").
    // For stop events, use last_message if available.
    let message = match event {
        HookEvent::PermissionRequest => {
            format_permission_request_message(input).unwrap_or_else(|| "Permission required".into())
        }
        _ => session
            .last_message
            .as_ref()
            .map(|m| truncate_string(m, 100))
            .unwrap_or_else(|| match event {
                HookEvent::Stop => "Session stopped".to_string(),
                _ => "Notification".to_string(),
            }),
    };

    let mut notification = Notification::new(&title, message);
    // Use configured sound; empty string means silent
    if !config.notification.sound.is_empty() {
        notification = notification.with_sound(&config.notification.sound);
    }

    if let Some(subtitle) = subtitle {
        notification = notification.with_subtitle(subtitle);
    }

    // Add click action to focus tmux pane if available
    // Skip action if paths cannot be safely quoted (e.g., contains null bytes)
    // Use full path for tmux because terminal-notifier's -execute runs in minimal PATH environment
    if let Some(tmux_info) = &session.tmux_info
        && let Ok(escaped_pane_id) = shlex::try_quote(&tmux_info.pane_id)
        && let Some(tmux_path) = find_command_path("tmux")
        && let Ok(tmux) = shlex::try_quote(&tmux_path.to_string_lossy())
    {
        // Use tmux switch-client with the first available client, then focus the configured app
        let focus_app_str = config.editor.focus_app();
        let focus_app =
            shlex::try_quote(focus_app_str).unwrap_or_else(|_| focus_app_str.to_string().into());
        let command = format!(
            r#"client_name=$({tmux} list-clients -F '#{{client_name}}' | head -n1); {tmux} switch-client -c "$client_name" -t {}; open -a {focus_app}"#,
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

    // Get session title: label (armyknife) > firstPrompt (Claude Code)
    let session_title = session
        .label
        .clone()
        .or_else(|| claude_sessions::get_session_title(&session.cwd, &session.session_id));

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
        create_test_input_with_source(notification_type, None)
    }

    fn create_test_input_with_source(
        notification_type: Option<&str>,
        source: Option<&str>,
    ) -> HookInput {
        let mut json_parts = vec![
            r#""session_id":"test-123""#.to_string(),
            r#""cwd":"/tmp/test""#.to_string(),
        ];
        if let Some(t) = notification_type {
            json_parts.push(format!(r#""notification_type":"{}""#, t));
        }
        if let Some(s) = source {
            json_parts.push(format!(r#""source":"{}""#, s));
        }
        let json = format!("{{{}}}", json_parts.join(","));
        serde_json::from_str(&json).expect("valid JSON")
    }

    fn create_test_input_with_tool(tool_name: &str, tool_input_json: Option<&str>) -> HookInput {
        let tool_input = match tool_input_json {
            Some(json) => format!(r#","tool_input":{}"#, json),
            None => String::new(),
        };
        let json = format!(
            r#"{{"session_id":"test-123","cwd":"/tmp/test","tool_name":"{}"{}}}"#,
            tool_name, tool_input
        );
        serde_json::from_str(&json).expect("valid JSON")
    }

    #[rstest]
    #[case::session_start(HookEvent::SessionStart, None, SessionStatus::Running)]
    #[case::user_prompt_submit(HookEvent::UserPromptSubmit, None, SessionStatus::Running)]
    #[case::pre_tool_use(HookEvent::PreToolUse, None, SessionStatus::Running)]
    #[case::post_tool_use(HookEvent::PostToolUse, None, SessionStatus::Running)]
    #[case::permission_request(HookEvent::PermissionRequest, None, SessionStatus::WaitingInput)]
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
            HookEvent::from_str("session-start").expect("valid event"),
            HookEvent::SessionStart
        );
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
            HookEvent::from_str("permission-request").expect("valid event"),
            HookEvent::PermissionRequest
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
    #[case::stop_always_notifies(HookEvent::Stop, true)]
    #[case::permission_request_notifies(HookEvent::PermissionRequest, true)]
    #[case::notification_no_notify(HookEvent::Notification, false)]
    #[case::user_prompt_no_notification(HookEvent::UserPromptSubmit, false)]
    #[case::pre_tool_no_notification(HookEvent::PreToolUse, false)]
    #[case::post_tool_no_notification(HookEvent::PostToolUse, false)]
    fn test_is_notifiable_event(#[case] event: HookEvent, #[case] expected: bool) {
        assert_eq!(is_notifiable_event(event), expected);
    }

    #[test]
    fn test_build_notification_stop_event() {
        let input = create_test_input(None);
        let mut session = create_test_session(None);
        session.status = SessionStatus::Stopped;
        let notification =
            build_notification(HookEvent::Stop, &input, &session, &Config::default());

        // Title includes status
        assert_eq!(notification.title(), "Claude Code - Stopped");
        // Message falls back to "Session stopped" when no last_message
        assert_eq!(notification.message(), "Session stopped");
        assert_eq!(notification.sound(), Some("Glass"));
        // No subtitle without tmux_info
        assert!(notification.subtitle().is_none());
        assert!(notification.action().is_none());
    }

    #[rstest]
    #[case::bash_command(
        "Bash",
        Some(r#"{"command":"cargo test --all"}"#),
        "Bash: cargo test --all"
    )]
    #[case::edit_file_path("Edit", Some(r#"{"file_path":"src/main.rs"}"#), "Edit: src/main.rs")]
    #[case::grep_pattern("Grep", Some(r#"{"pattern":"TODO"}"#), "Grep: TODO")]
    #[case::task_no_input("Task", None, "Task")]
    fn test_build_notification_permission_request_message(
        #[case] tool_name: &str,
        #[case] tool_input_json: Option<&str>,
        #[case] expected_message: &str,
    ) {
        let input = create_test_input_with_tool(tool_name, tool_input_json);
        let mut session = create_test_session(None);
        session.status = SessionStatus::WaitingInput;
        let notification = build_notification(
            HookEvent::PermissionRequest,
            &input,
            &session,
            &Config::default(),
        );

        assert_eq!(notification.title(), "Claude Code - Waiting");
        assert_eq!(notification.message(), expected_message);
    }

    #[test]
    fn test_build_notification_permission_request_ignores_last_message() {
        let input = create_test_input_with_tool("Bash", Some(r#"{"command":"rm -rf /tmp/test"}"#));
        let mut session = create_test_session(None);
        session.status = SessionStatus::WaitingInput;
        // Even when last_message exists, permission_request should use tool details
        session.last_message = Some("I'll clean up the temp files.".to_string());
        let notification = build_notification(
            HookEvent::PermissionRequest,
            &input,
            &session,
            &Config::default(),
        );

        // Should show tool details, not last assistant message
        assert_eq!(notification.message(), "Bash: rm -rf /tmp/test");
    }

    #[test]
    fn test_build_notification_permission_request_fallback() {
        // Input without tool_name (edge case)
        let input = create_test_input(None);
        let mut session = create_test_session(None);
        session.status = SessionStatus::WaitingInput;
        let notification = build_notification(
            HookEvent::PermissionRequest,
            &input,
            &session,
            &Config::default(),
        );

        // Falls back to generic message
        assert_eq!(notification.message(), "Permission required");
    }

    #[test]
    fn test_build_notification_with_last_message() {
        let input = create_test_input(None);
        let mut session = create_test_session(None);
        session.status = SessionStatus::Stopped;
        session.last_message = Some("I've updated the code as requested.".to_string());
        let notification =
            build_notification(HookEvent::Stop, &input, &session, &Config::default());

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
        let notification =
            build_notification(HookEvent::Stop, &input, &session, &Config::default());

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
    fn test_build_notification_custom_sound() {
        let input = create_test_input(None);
        let mut session = create_test_session(None);
        session.status = SessionStatus::Stopped;
        let mut config = Config::default();
        config.notification.sound = "Ping".to_string();
        let notification = build_notification(HookEvent::Stop, &input, &session, &config);

        assert_eq!(notification.sound(), Some("Ping"));
    }

    #[test]
    fn test_build_notification_silent_sound() {
        let input = create_test_input(None);
        let mut session = create_test_session(None);
        session.status = SessionStatus::Stopped;
        let mut config = Config::default();
        config.notification.sound = String::new();
        let notification = build_notification(HookEvent::Stop, &input, &session, &config);

        // Empty string means no sound
        assert!(notification.sound().is_none());
    }

    #[test]
    fn test_build_notification_custom_focus_app() {
        let input = create_test_input(None);
        let mut session = create_test_session(Some(TmuxInfo {
            session_name: "main".to_string(),
            window_name: "dev".to_string(),
            window_index: 1,
            pane_id: "%123".to_string(),
        }));
        session.status = SessionStatus::Stopped;
        let mut config = Config::default();
        config.editor.focus_app = Some("Alacritty".to_string());
        let notification = build_notification(HookEvent::Stop, &input, &session, &config);

        // Action command should use the configured focus_app
        assert!(notification.action().is_some());
        let action = notification.action().expect("action present");
        assert!(
            action.command().contains("open -a Alacritty"),
            "expected 'open -a Alacritty' in command, got: {}",
            action.command()
        );
    }

    #[test]
    fn test_should_notify_respects_config_disabled() {
        let mut config = Config::default();
        config.notification.enabled = false;

        assert!(!should_notify(HookEvent::Stop, &config));
        assert!(!should_notify(HookEvent::PermissionRequest, &config));
    }

    #[test]
    fn test_should_notify_respects_config_enabled() {
        let config = Config::default();

        // Default config has notifications enabled
        assert!(should_notify(HookEvent::Stop, &config));
        assert!(should_notify(HookEvent::PermissionRequest, &config));
        // Non-notifiable events still return false
        assert!(!should_notify(HookEvent::UserPromptSubmit, &config));
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
            label: None,
            ancestor_session_ids: Vec::new(),
        }
    }

    mod hook_log_tests {
        use super::*;
        use rstest::rstest;
        use tempfile::TempDir;

        #[rstest]
        #[case::success_log(true, None)]
        #[case::error_log(false, Some("parse error"))]
        fn creates_hook_log_file(#[case] success: bool, #[case] error_message: Option<&str>) {
            let temp_dir = TempDir::new().expect("temp dir creation should succeed");
            let logs_dir = temp_dir.path().to_path_buf();

            let stdin_content = r#"{"session_id": "test-123"}"#;
            let event = "pre-tool-use";

            let log_path =
                write_hook_log_to_dir(stdin_content, event, success, error_message, &logs_dir)
                    .expect("should succeed");

            assert!(log_path.exists(), "hook log file should be created");

            let written = fs::read_to_string(&log_path).expect("should read log file");

            let expected = if let Some(msg) = error_message {
                formatdoc! {"
                    === Event ===
                    {event}

                    === Status ===
                    error

                    === Error Message ===
                    {msg}

                    === Raw Stdin ===
                    {stdin_content}"}
            } else {
                formatdoc! {"
                    === Event ===
                    {event}

                    === Status ===
                    success

                    === Raw Stdin ===
                    {stdin_content}"}
            };
            assert_eq!(written, expected);
        }

        #[test]
        fn hook_log_filename_format() {
            let temp_dir = TempDir::new().expect("temp dir creation should succeed");
            let logs_dir = temp_dir.path().to_path_buf();

            let log_path = write_hook_log_to_dir("content", "stop", true, None, &logs_dir)
                .expect("should succeed");
            let filename = log_path
                .file_name()
                .expect("should have filename")
                .to_string_lossy();

            assert!(
                filename.starts_with("hook_"),
                "expected to start with 'hook_', got: {filename}"
            );
            assert!(
                filename.ends_with(".log"),
                "expected to end with '.log', got: {filename}"
            );
        }

        #[cfg(unix)]
        #[test]
        fn hook_log_has_restrictive_permissions() {
            use std::os::unix::fs::PermissionsExt;

            let temp_dir = TempDir::new().expect("temp dir creation should succeed");
            let logs_dir = temp_dir.path().to_path_buf();

            let log_path = write_hook_log_to_dir("content", "stop", true, None, &logs_dir)
                .expect("should succeed");
            let metadata = fs::metadata(&log_path).expect("should get metadata");
            let mode = metadata.permissions().mode() & 0o777;

            assert_eq!(mode, 0o600, "hook log file should have 0600 permissions");
        }

        #[test]
        fn logs_dir_uses_cache_directory() {
            let logs = logs_dir().expect("should have cache directory");
            assert!(
                logs.ends_with("cc/logs"),
                "expected to end with 'cc/logs', got: {logs:?}"
            );
        }
    }

    mod session_start_tests {
        use super::*;
        use rstest::rstest;
        use tempfile::TempDir;

        /// Creates a temporary directory for session storage.
        fn create_temp_sessions_dir() -> TempDir {
            TempDir::new().expect("temp dir creation should succeed")
        }

        #[rstest]
        #[case::startup_skips(Some("startup"), ProcessResult::Skipped)]
        #[case::resume_creates_session(Some("resume"), ProcessResult::SessionSaved)]
        #[case::none_creates_session(None, ProcessResult::SessionSaved)]
        fn session_start_with_source(
            #[case] source: Option<&str>,
            #[case] expected_result: ProcessResult,
        ) {
            let temp_dir = create_temp_sessions_dir();

            // When `claude -c` resumes a session, Claude Code fires two SessionStart hooks:
            // - "startup" with a new (unwanted) session_id -> should be skipped
            // - "resume" with the actual session_id -> should create session
            let input = create_test_input_with_source(None, source);

            let result = process_hook_event_impl(HookEvent::SessionStart, input, temp_dir.path())
                .expect("should succeed");

            assert_eq!(
                result, expected_result,
                "source={:?} should return {:?}",
                source, expected_result
            );
        }

        #[test]
        fn claude_c_resume_scenario() {
            let temp_dir = create_temp_sessions_dir();

            // Simulate `claude -c` which fires two SessionStart hooks in quick succession.
            // The "startup" event should be skipped, "resume" should create the session.

            // First event: "startup" with a new (unwanted) session_id
            let startup_input = create_test_input_with_source(None, Some("startup"));
            let startup_result =
                process_hook_event_impl(HookEvent::SessionStart, startup_input, temp_dir.path())
                    .expect("startup should succeed");
            assert_eq!(
                startup_result,
                ProcessResult::Skipped,
                "startup event should be skipped to prevent empty session creation"
            );

            // Second event: "resume" with the actual session_id being restored
            let resume_input = create_test_input_with_source(None, Some("resume"));
            let resume_result =
                process_hook_event_impl(HookEvent::SessionStart, resume_input, temp_dir.path())
                    .expect("resume should succeed");
            assert_eq!(
                resume_result,
                ProcessResult::SessionSaved,
                "resume event should create the session"
            );
        }

        #[test]
        fn new_session_without_c_flag() {
            let temp_dir = create_temp_sessions_dir();

            // Simulate `claude` (without -c) which fires only one SessionStart hook.
            // Since source="startup", the session is NOT created on SessionStart.
            // Session will be created on first user-prompt-submit instead.
            let input = create_test_input_with_source(None, Some("startup"));

            let result = process_hook_event_impl(HookEvent::SessionStart, input, temp_dir.path())
                .expect("should succeed");

            assert_eq!(
                result,
                ProcessResult::Skipped,
                "new session startup should be skipped; session created on user-prompt-submit"
            );
        }

        #[test]
        fn user_prompt_submit_creates_session() {
            let temp_dir = create_temp_sessions_dir();

            // Verify that user-prompt-submit creates a session (for new sessions that
            // skipped SessionStart due to source="startup")
            let input = create_test_input(None);

            let result =
                process_hook_event_impl(HookEvent::UserPromptSubmit, input, temp_dir.path())
                    .expect("should succeed");

            assert_eq!(
                result,
                ProcessResult::SessionSaved,
                "user-prompt-submit should create the session"
            );
        }
    }

    mod log_level_tests {
        use super::*;
        use rstest::rstest;

        #[rstest]
        #[case::debug(Some("debug"), LogLevel::Debug)]
        #[case::error(Some("error"), LogLevel::Error)]
        #[case::off(Some("off"), LogLevel::Off)]
        #[case::unknown(Some("unknown"), LogLevel::Error)]
        #[case::empty(Some(""), LogLevel::Error)]
        #[case::not_set(None, LogLevel::Error)]
        fn log_level_from_str(#[case] value: Option<&str>, #[case] expected: LogLevel) {
            assert_eq!(LogLevel::from_str(value), expected);
        }

        #[rstest]
        #[case::debug(LogLevel::Debug, true)]
        #[case::error(LogLevel::Error, true)]
        #[case::off(LogLevel::Off, false)]
        fn should_log_errors(#[case] level: LogLevel, #[case] expected: bool) {
            assert_eq!(level.should_log_errors(), expected);
        }
    }

    mod transcript_retry_tests {
        use super::*;
        use std::cell::Cell;
        use std::time::Duration;

        /// Zero delay for fast tests
        const TEST_DELAY: Duration = Duration::from_millis(0);

        #[test]
        fn returns_immediately_when_no_previous_message() {
            let call_count = Cell::new(0u32);
            let result = get_last_message_with_retry_impl(None, TEST_DELAY, 5, || {
                call_count.set(call_count.get() + 1);
                Some("new message".to_string())
            });

            assert_eq!(result, Some("new message".to_string()));
            assert_eq!(call_count.get(), 1, "should read transcript exactly once");
        }

        #[test]
        fn returns_immediately_when_message_changed() {
            let call_count = Cell::new(0u32);
            let result =
                get_last_message_with_retry_impl(Some("old message"), TEST_DELAY, 5, || {
                    call_count.set(call_count.get() + 1);
                    Some("new message".to_string())
                });

            assert_eq!(result, Some("new message".to_string()));
            assert_eq!(call_count.get(), 1, "should read transcript exactly once");
        }

        #[test]
        fn retries_when_message_unchanged_then_succeeds() {
            let call_count = Cell::new(0u32);
            let result =
                get_last_message_with_retry_impl(Some("old message"), TEST_DELAY, 5, || {
                    call_count.set(call_count.get() + 1);
                    if call_count.get() <= 2 {
                        // First read + first retry: transcript not yet updated
                        Some("old message".to_string())
                    } else {
                        // Second retry: transcript updated
                        Some("new message".to_string())
                    }
                });

            assert_eq!(result, Some("new message".to_string()));
            // 1 initial read + 2 retries (first retry returns old, second returns new)
            assert_eq!(call_count.get(), 3);
        }

        #[test]
        fn returns_old_message_after_max_retries() {
            let call_count = Cell::new(0u32);
            let result =
                get_last_message_with_retry_impl(Some("same message"), TEST_DELAY, 3, || {
                    call_count.set(call_count.get() + 1);
                    Some("same message".to_string())
                });

            assert_eq!(result, Some("same message".to_string()));
            // 1 initial read + 3 retries
            assert_eq!(call_count.get(), 4);
        }

        #[test]
        fn returns_none_when_transcript_empty_and_no_previous() {
            let result = get_last_message_with_retry_impl(None, TEST_DELAY, 5, || None);

            assert!(result.is_none());
        }

        #[test]
        fn returns_none_immediately_when_transcript_becomes_empty() {
            // Previous message existed but transcript now returns None (different from previous)
            let call_count = Cell::new(0u32);
            let result =
                get_last_message_with_retry_impl(Some("old message"), TEST_DELAY, 5, || {
                    call_count.set(call_count.get() + 1);
                    None
                });

            assert!(result.is_none());
            assert_eq!(call_count.get(), 1, "should not retry when result differs");
        }

        #[test]
        fn zero_max_retries_reads_once() {
            let call_count = Cell::new(0u32);
            let result =
                get_last_message_with_retry_impl(Some("old message"), TEST_DELAY, 0, || {
                    call_count.set(call_count.get() + 1);
                    Some("old message".to_string())
                });

            assert_eq!(result, Some("old message".to_string()));
            assert_eq!(
                call_count.get(),
                1,
                "should read exactly once with 0 retries"
            );
        }
    }
}
