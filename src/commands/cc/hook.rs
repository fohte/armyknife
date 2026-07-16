use std::collections::BTreeSet;
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

use super::auto_compact;
use super::claude_sessions;
use super::error::CcError;
use super::store;
use super::tmux_sync::{LiveTmuxStatusSyncer, TmuxStatusSyncer};
use super::types::{HookEvent, HookInput, Session, SessionStatus, TMUX_SESSION_OPTION, TmuxInfo};
use crate::infra::notification::{Notification, NotificationAction};
use crate::infra::tmux;
use crate::shared::cache;
use crate::shared::config::{self, Config, Terminal};
use crate::shared::env_var::EnvVars;
use crate::shared::log::short_run_id;

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

    let run_id = short_run_id();
    let span = tracing::info_span!("cc.hook", run_id = %run_id, event = %args.event);
    let _entered = span.enter();

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

/// Controls which side effects `process_hook_event_impl` executes.
/// Production code uses `SideEffects::all()`; tests use `SideEffects::none()`
/// to avoid calling external commands (tmux, hammerspoon, etc.).
struct SideEffects {
    /// Call tmux commands (get_pane_info_by_pid, set_pane_option, refresh_status)
    tmux: bool,
    /// Send/remove notifications via hammerspoon
    notifications: bool,
    /// Spawn the detached `a cc auto-compact schedule` worker on Stop events.
    /// Off in tests (would fork a real process and survive past the test).
    auto_compact: bool,
    /// Test-only sink that records the group ids passed to
    /// `remove_notification_group`. Lets tests assert the call happened
    /// without invoking hammerspoon.
    #[cfg(test)]
    removed_notification_groups: Option<std::sync::Arc<std::sync::Mutex<Vec<String>>>>,
    /// Test-only sink that records (pane_id, status, sessions_dir) tuples
    /// passed to `sync_tmux`. Lets tests assert the call happened with the
    /// expected status without invoking tmux.
    #[cfg(test)]
    tmux_sync_calls: Option<TmuxSyncCallSink>,
}

#[cfg(test)]
type TmuxSyncCall = (Option<String>, Option<SessionStatus>, std::path::PathBuf);
#[cfg(test)]
type TmuxSyncCallSink = std::sync::Arc<std::sync::Mutex<Vec<TmuxSyncCall>>>;

impl SideEffects {
    fn all() -> Self {
        Self {
            tmux: true,
            notifications: true,
            auto_compact: true,
            #[cfg(test)]
            removed_notification_groups: None,
            #[cfg(test)]
            tmux_sync_calls: None,
        }
    }

    #[cfg(test)]
    fn none() -> Self {
        Self {
            tmux: false,
            notifications: false,
            auto_compact: false,
            removed_notification_groups: None,
            tmux_sync_calls: None,
        }
    }

    /// Pushes the latest pane / window status into tmux. In tests, also
    /// records the call into `tmux_sync_calls` so assertions don't require
    /// real tmux.
    fn sync_tmux(&self, pane_id: Option<&str>, status: Option<SessionStatus>, sessions_dir: &Path) {
        #[cfg(test)]
        if let Some(rec) = &self.tmux_sync_calls {
            rec.lock().expect("tmux_sync_calls mutex poisoned").push((
                pane_id.map(str::to_string),
                status,
                sessions_dir.to_path_buf(),
            ));
        }
        if self.tmux {
            LiveTmuxStatusSyncer.sync(pane_id, status, sessions_dir);
        }
    }

    fn remove_notification_group(&self, group: &str) {
        if self.notifications {
            let _ = crate::infra::notification::remove_group(group);
        }
        #[cfg(test)]
        if let Some(rec) = &self.removed_notification_groups {
            rec.lock()
                .expect("removed_notification_groups mutex poisoned")
                .push(group.to_string());
        }
    }
}

/// Processes a hook event with the given input.
/// This is the core logic separated from stdin handling for testability.
fn process_hook_event(event: HookEvent, input: HookInput) -> Result<()> {
    let sessions_dir = store::sessions_dir()?;
    process_hook_event_impl(event, input, &sessions_dir, &SideEffects::all()).map(|_| ())
}

/// Ends any Paused sessions that were attached to `pane_id` but belong to a
/// different `session_id` than the one now taking the pane. Without this, a
/// session auto-paused by `a cc sweep` lingers in `a cc watch` after another
/// `claude` (or `claude -c <other-id>`) starts on the same pane. Resuming the
/// same paused session with `claude -c <same-id>` is unaffected because
/// `session_id` matches and the entry is skipped.
fn evict_paused_sessions_on_pane_takeover(
    sessions_dir: &Path,
    pane_id: &str,
    current_session_id: &str,
) {
    let Ok(entries) = fs::read_dir(sessions_dir) else {
        return;
    };
    let now = Utc::now();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(mut session) = serde_json::from_str::<Session>(&content) else {
            continue;
        };
        let matches_pane = session
            .tmux_info
            .as_ref()
            .is_some_and(|info| info.pane_id == pane_id);
        if session.status != SessionStatus::Paused
            || session.session_id == current_session_id
            || !matches_pane
        {
            continue;
        }
        session.status = SessionStatus::Ended;
        session.updated_at = now;
        let _ = store::save_session_to(sessions_dir, &session);
    }
}

/// Internal implementation that returns ProcessResult for testing.
/// Accepts sessions_dir as a parameter to allow testing with temporary directories.
fn process_hook_event_impl(
    event: HookEvent,
    input: HookInput,
    sessions_dir: &Path,
    side_effects: &SideEffects,
) -> Result<ProcessResult> {
    let env = EnvVars::load();

    // Handle session end: mark as ended instead of deleting so that
    // `claude -c` resume can restore label and ancestor chain.
    // Ended sessions are garbage-collected by cleanup_stale_sessions.
    //
    // Paused sessions are preserved: when `a cc sweep` SIGTERMs a stopped
    // Claude Code process, its shutdown fires SessionEnd, which would
    // otherwise clobber the Paused marker and break `a cc resume`.
    if event == HookEvent::SessionEnd {
        // When the session file is gone the pane_id is unrecoverable
        // (hook input only carries session_id), so we silently drop the
        // SessionEnd. The pane option is keyed off the pane's bound
        // session_id and will be reconciled the next time that pane fires
        // a hook for the new session.
        if let Some(mut session) = store::load_session_from(sessions_dir, &input.session_id)? {
            let pane_id = session.tmux_info.as_ref().map(|info| info.pane_id.clone());
            if session.status != SessionStatus::Paused {
                session.status = SessionStatus::Ended;
                session.updated_at = Utc::now();
                store::save_session_to(sessions_dir, &session)?;
                // The user terminated this session (sweep flips status to
                // Paused before SIGTERM, so reaching this branch means a
                // Ctrl-C / `/exit` / crash, not a sweep). No further events
                // will arrive to auto-clear lingering notifications, so do it
                // here. Paused sessions keep their notification so the user
                // still sees it after `a cc resume`.
                side_effects.remove_notification_group(&input.session_id);
            }
            // Push the preserved status into the pane option so that sweep's
            // Paused isn't clobbered back to "" by this SessionEnd: Paused
            // for sweep auto-pauses, Ended otherwise.
            side_effects.sync_tmux(pane_id.as_deref(), Some(session.status), sessions_dir);
        }
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

        if side_effects.tmux
            && let Some(pane_info) = tmux::get_pane_info_by_pid(std::process::id())
        {
            // Ignore errors; pane option is nice-to-have, not critical
            let _ =
                tmux::set_pane_option(&pane_info.pane_id, TMUX_SESSION_OPTION, &input.session_id);
            evict_paused_sessions_on_pane_takeover(
                sessions_dir,
                &pane_info.pane_id,
                &input.session_id,
            );
        }
    }

    // Set pane option on UserPromptSubmit for new sessions.
    // When `claude` is started without `-c`, only SessionStart(startup) fires,
    // which skips setting the pane option to avoid wrong session_id on resume.
    // UserPromptSubmit is the earliest subsequent event where we can set it.
    // Skip once the session file exists: pane option and eviction only need to
    // run at the moment of pane handover (the first prompt of a new session),
    // and re-running them on every prompt costs an O(N) disk scan plus a
    // `tmux` process spawn for no behavioral effect.
    if side_effects.tmux
        && event == HookEvent::UserPromptSubmit
        && !sessions_dir
            .join(format!("{}.json", input.session_id))
            .exists()
        && let Some(pane_info) = tmux::get_pane_info_by_pid(std::process::id())
    {
        let _ = tmux::set_pane_option(&pane_info.pane_id, TMUX_SESSION_OPTION, &input.session_id);
        evict_paused_sessions_on_pane_takeover(sessions_dir, &pane_info.pane_id, &input.session_id);
    }

    // Get tmux info by finding the pane that contains this process
    let tmux_info = if side_effects.tmux {
        tmux::get_pane_info_by_pid(std::process::id()).map(|info| TmuxInfo {
            session_name: info.session_name,
            window_name: info.window_name,
            window_index: info.window_index,
            pane_id: info.pane_id,
        })
    } else {
        None
    };

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
                pending_bg_task_ids: BTreeSet::new(),
                read_at: None,
            }
        });

    // Update session fields
    session.cwd.clone_from(&input.cwd);
    session.updated_at = now;

    // Preserve Paused during SIGTERM shutdown: when sweep SIGTERMs a stopped
    // Claude, its shutdown may fire Stop hooks that would overwrite Paused →
    // Stopped, and then SessionEnd would see Stopped instead of Paused.
    // However, when the user resumes (SessionStart(resume) → Stopped, same
    // value a Stop hook would produce), the status must still transition out
    // of Paused so the TUI shows the session as active and `a cc sweep` can
    // re-arm its idle timeout. Excluding SessionStart here is what
    // distinguishes "sweep's own shutdown Stop" from "the user resumed".
    let keep_paused = session.status == SessionStatus::Paused
        && event != HookEvent::SessionStart
        && matches!(status, SessionStatus::Stopped | SessionStatus::Ended);
    if !keep_paused {
        session.status = status;
    }

    if session.status == SessionStatus::Stopped {
        session.read_at = None;
    }

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

    // Track in-flight background tasks (Bash `run_in_background: true`).
    // The immediately-following Stop fires synthetically — Claude moves on
    // as soon as the bg task is spawned, not when it finishes — and there
    // is no completion hook for the bg task itself. We accumulate ids here
    // so that:
    //
    // - `auto_compact` skips the synthetic Stop while any id is pending.
    // - `sweep` does not SIGTERM the session while any id is pending
    //   (otherwise a long bg task gets killed mid-flight).
    //
    // Removal is driven by PostToolUse for `BashOutput` / `KillShell`,
    // which observe completion / explicit kill respectively. Schema for
    // those tool_response payloads is not documented
    // (anthropics/claude-code#3671); we treat any string `status` matching
    // a known terminal value as completion and silently ignore everything
    // else. Stale ids left behind by schema drift are harmless: they only
    // delay auto-compact / auto-pause until the user explicitly resumes
    // the session (which initializes a fresh set on the next session
    // creation cycle).
    if event == HookEvent::PostToolUse {
        update_pending_bg_tasks(&mut session, &input);
    }

    // Save updated session
    store::save_session_to(sessions_dir, &session)?;

    // Push the window's aggregated Claude Code status into its
    // `@armyknife-cc-window-status` tmux option. The write and the status-bar refresh
    // are skipped when the rendered value is unchanged, so an event that does
    // not alter the visible status (e.g. running → running) costs no redraw.
    side_effects.sync_tmux(
        session.tmux_info.as_ref().map(|info| info.pane_id.as_str()),
        Some(session.status),
        sessions_dir,
    );

    // Remove stale notifications on every event that reaches here, except
    // Notification events.  Notification(permission_prompt) fires right after
    // PermissionRequest for the same permission ask; clearing the group there
    // would erase the just-sent notification before the user sees it.
    // SessionEnd never reaches here (early return above).
    if !matches!(event, HookEvent::Notification) {
        side_effects.remove_notification_group(&input.session_id);
    }

    // Send notification if applicable (errors are logged but don't fail the hook).
    // Use default config if loading fails to avoid config errors blocking notifications.
    if side_effects.notifications {
        let config = config::load_config().unwrap_or_default();
        if should_notify(event, &config) {
            send_notification(event, &input, &session, &config);
        }
    }

    // On Stop events, spawn a detached schedule worker that will SIGTERM +
    // `claude -r -p "/compact"` after the configured idle timeout. The
    // worker re-checks user activity / branch state on wake-up so a quick
    // follow-up turn cancels the compaction transparently.
    //
    // Skip while any background task launched in this session has not
    // reported completion: the user is still mid-task even if Claude's
    // main loop went idle (the post-launch Stop is synthetic). Ids are
    // removed lazily by BashOutput / KillShell PostToolUse hooks, not
    // cleared here, so a turn that mixes a bg launch with a real Stop is
    // still suppressed correctly.
    if side_effects.auto_compact && event == HookEvent::Stop {
        if !session.pending_bg_task_ids.is_empty() {
            tracing::info!(
                event = "cc.auto_compact.skipped",
                session = %session.session_id,
                reason = "bg_task_pending",
                pending = session.pending_bg_task_ids.len(),
            );
        } else {
            let config = config::load_config().unwrap_or_default();
            if config.cc.auto_compact.enabled {
                auto_compact::spawn_in_background(&session.session_id);
            } else {
                tracing::info!(
                    event = "cc.auto_compact.skipped",
                    session = %session.session_id,
                    reason = "disabled",
                );
            }
        }
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
        // A resumed session (`claude -r` / `claude -c` / `/resume`) is waiting
        // for input just like a session that already hit Stop: if the user
        // sends no prompt, no further hook will ever fire to correct the
        // status. Reporting Running here would leave the session stuck
        // forever, invisible to `a cc sweep`'s idle-timeout check.
        HookEvent::SessionStart if input.source.as_deref() == Some("resume") => {
            SessionStatus::Stopped
        }
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
    // Title: "⏳ Claude Code - Waiting" or "⏹ Claude Code - Stopped"
    let (emoji, status_label) = match session.status {
        SessionStatus::WaitingInput => ("\u{23f3}", "Waiting"),
        SessionStatus::Stopped => ("\u{23f9}", "Stopped"),
        SessionStatus::Running => ("\u{25b6}\u{fe0f}", "Running"),
        SessionStatus::Paused => ("\u{23f8}", "Paused"),
        SessionStatus::Ended => ("\u{1f3c1}", "Ended"),
    };
    let title = format!("{} Claude Code - {}", emoji, status_label);

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

    // Set group ID to session ID for notification management (replace/remove)
    notification = notification.with_group(&session.session_id);

    // Set app icon for visual identification
    if let Some(icon_path) = crate::infra::notification::icon::ensure_icon()
        && let Some(path_str) = icon_path.to_str()
    {
        notification = notification.with_app_icon(path_str);
    }

    // Use configured sound; empty string means silent
    if !config.notification.sound.is_empty() {
        notification = notification.with_sound(&config.notification.sound);
    }

    if let Some(subtitle) = subtitle {
        notification = notification.with_subtitle(subtitle);
    }

    // Add click action to focus tmux pane via `a cc focus` + app focus
    if session.tmux_info.is_some() {
        let session_id = shlex::try_quote(&session.session_id)
            .unwrap_or_else(|_| session.session_id.clone().into());
        let focus_cmd = build_focus_app_command(config);
        let command = format!("a cc focus {session_id}; {focus_cmd}");
        notification = notification.with_action(NotificationAction::new(command));
    }

    notification
}

/// Ghostty's default window title. Used to identify the main terminal window
/// when focusing via AppleScript, since Ghostty's AppleScript API does not
/// expose tty information per window (https://github.com/ghostty-org/ghostty/issues/10756).
const GHOSTTY_DEFAULT_TITLE: &str = "👻";

/// Builds a shell command to focus the terminal application.
/// For Ghostty on macOS, uses AppleScript to focus the main window by its default title.
/// For other terminals, uses `open -a` which activates the most recent window.
fn build_focus_app_command(config: &Config) -> String {
    if cfg!(target_os = "macos")
        && config.editor.terminal == Terminal::Ghostty
        && config.editor.focus_app.is_none()
    {
        format!(
            "osascript -e 'tell application \"Ghostty\"' -e 'activate (first window whose name is \"{GHOSTTY_DEFAULT_TITLE}\")' -e 'activate' -e 'end tell'"
        )
    } else {
        let focus_app_str = config.editor.focus_app();
        let focus_app =
            shlex::try_quote(focus_app_str).unwrap_or_else(|_| focus_app_str.to_string().into());
        format!("open -a {focus_app}")
    }
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

/// Updates `session.pending_bg_task_ids` based on a `PostToolUse` event.
///
/// - `Bash` launch with `run_in_background: true`: tool_response carries
///   `backgroundTaskId`; insert it.
/// - `BashOutput`: tool_response carries `status`; remove the referenced
///   shell id when status is "completed", "killed", or "failed". Any other
///   status (running, missing, schema mismatch) leaves the set alone — see
///   the call-site comment for why stale ids are acceptable.
/// - `KillShell`: shell id is removed unconditionally; the tool's whole
///   purpose is to terminate the bg task.
fn update_pending_bg_tasks(session: &mut Session, input: &HookInput) {
    if let Some(bg_id) = input.background_task_id() {
        session.pending_bg_task_ids.insert(bg_id.to_string());
        return;
    }

    let tool_name = input.tool_name.as_deref();
    match tool_name {
        Some("BashOutput") => {
            let Some(shell_id) = input.shell_id() else {
                return;
            };
            if matches!(
                input.bash_output_status(),
                Some("completed" | "killed" | "failed")
            ) {
                session.pending_bg_task_ids.remove(shell_id);
            }
        }
        Some("KillShell") => {
            if let Some(shell_id) = input.shell_id() {
                session.pending_bg_task_ids.remove(shell_id);
            }
        }
        _ => {}
    }
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
    #[case::session_start(HookEvent::SessionStart, None, None, SessionStatus::Running)]
    #[case::session_start_startup(
        HookEvent::SessionStart,
        None,
        Some("startup"),
        SessionStatus::Running
    )]
    #[case::session_start_resume(
        HookEvent::SessionStart,
        None,
        Some("resume"),
        SessionStatus::Stopped
    )]
    #[case::session_start_clear(
        HookEvent::SessionStart,
        None,
        Some("clear"),
        SessionStatus::Running
    )]
    #[case::session_start_compact(
        HookEvent::SessionStart,
        None,
        Some("compact"),
        SessionStatus::Running
    )]
    #[case::user_prompt_submit(HookEvent::UserPromptSubmit, None, None, SessionStatus::Running)]
    #[case::pre_tool_use(HookEvent::PreToolUse, None, None, SessionStatus::Running)]
    #[case::post_tool_use(HookEvent::PostToolUse, None, None, SessionStatus::Running)]
    #[case::permission_request(
        HookEvent::PermissionRequest,
        None,
        None,
        SessionStatus::WaitingInput
    )]
    #[case::stop(HookEvent::Stop, None, None, SessionStatus::Stopped)]
    #[case::notification_generic(
        HookEvent::Notification,
        Some("info"),
        None,
        SessionStatus::Running
    )]
    #[case::notification_permission(
        HookEvent::Notification,
        Some("permission_prompt"),
        None,
        SessionStatus::WaitingInput
    )]
    #[case::notification_idle(
        HookEvent::Notification,
        Some("idle_prompt"),
        None,
        SessionStatus::Stopped
    )]
    fn test_determine_status(
        #[case] event: HookEvent,
        #[case] notification_type: Option<&str>,
        #[case] source: Option<&str>,
        #[case] expected: SessionStatus,
    ) {
        let input = create_test_input_with_source(notification_type, source);
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

        // Title includes emoji and status
        assert_eq!(notification.title(), "\u{23f9} Claude Code - Stopped");
        // Message falls back to "Session stopped" when no last_message
        assert_eq!(notification.message(), "Session stopped");
        assert_eq!(notification.sound(), Some("Glass"));
        // No subtitle without tmux_info
        assert!(notification.subtitle().is_none());
        assert!(notification.action().is_none());
        // Group is set to session ID
        assert_eq!(notification.group(), Some("test-123"));
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

        assert_eq!(notification.title(), "\u{23f3} Claude Code - Waiting");
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

    #[rstest]
    #[case::running(SessionStatus::Running, "\u{25b6}\u{fe0f} Claude Code - Running")]
    #[case::ended(SessionStatus::Ended, "\u{1f3c1} Claude Code - Ended")]
    #[case::waiting(SessionStatus::WaitingInput, "\u{23f3} Claude Code - Waiting")]
    #[case::stopped(SessionStatus::Stopped, "\u{23f9} Claude Code - Stopped")]
    fn test_build_notification_emoji_title(
        #[case] status: SessionStatus,
        #[case] expected_title: &str,
    ) {
        let input = create_test_input(None);
        let mut session = create_test_session(None);
        session.status = status;
        let notification = build_notification(
            HookEvent::Notification,
            &input,
            &session,
            &Config::default(),
        );

        assert_eq!(notification.title(), expected_title);
        assert_eq!(notification.group(), Some("test-123"));
    }

    #[test]
    fn test_build_notification_fallback_message() {
        let input = create_test_input(None);
        let mut session = create_test_session(None);
        session.status = SessionStatus::Running;
        let notification = build_notification(
            HookEvent::Notification,
            &input,
            &session,
            &Config::default(),
        );

        // Non-stop, non-permission events without last_message fall back to "Notification"
        assert_eq!(notification.message(), "Notification");
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

        // Action should focus the session pane and activate the terminal app
        assert!(notification.action().is_some());
        let action = notification.action().expect("action present");
        assert!(action.command().contains("a cc focus"));
        assert!(action.command().contains("WezTerm"));
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

    #[rstest]
    #[case::stop_preserves_paused(HookEvent::Stop, None, SessionStatus::Paused)]
    #[case::pre_tool_use_transitions_to_running(
        HookEvent::PreToolUse,
        None,
        SessionStatus::Running
    )]
    #[case::user_prompt_submit_transitions_to_running(
        HookEvent::UserPromptSubmit,
        None,
        SessionStatus::Running
    )]
    #[case::session_start_resume_exits_paused(
        HookEvent::SessionStart,
        Some("resume"),
        SessionStatus::Stopped
    )]
    fn hook_on_paused_session(
        #[case] event: HookEvent,
        #[case] source: Option<&str>,
        #[case] expected: SessionStatus,
    ) {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let sessions_dir = temp_dir.path();

        let session = Session {
            session_id: "paused-sess".to_string(),
            cwd: "/tmp/test".into(),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Paused,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: BTreeSet::new(),
            read_at: None,
        };
        store::save_session_to(sessions_dir, &session).expect("save");

        let source_field = source
            .map(|s| format!(r#","source":"{s}""#))
            .unwrap_or_default();
        let input: HookInput = serde_json::from_str(&format!(
            r#"{{"session_id":"paused-sess","cwd":"/tmp/test"{source_field}}}"#
        ))
        .expect("valid JSON");

        process_hook_event_impl(event, input, sessions_dir, &SideEffects::none())
            .expect("hook should succeed");

        let reloaded = store::load_session_from(sessions_dir, "paused-sess")
            .expect("load")
            .expect("session exists");
        assert_eq!(
            reloaded.status, expected,
            "{event:?} on Paused session should result in {expected:?}"
        );
    }

    #[rstest]
    #[case::user_ended_clears_notification(SessionStatus::Running, vec!["end-sess".to_string()])]
    #[case::sweep_paused_keeps_notification(SessionStatus::Paused, Vec::<String>::new())]
    fn session_end_clears_notification_only_when_not_paused(
        #[case] initial_status: SessionStatus,
        #[case] expected_removed: Vec<String>,
    ) {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let sessions_dir = temp_dir.path();

        let session = Session {
            session_id: "end-sess".to_string(),
            cwd: "/tmp/test".into(),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: initial_status,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: BTreeSet::new(),
            read_at: None,
        };
        store::save_session_to(sessions_dir, &session).expect("save");

        let removed = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let side_effects = SideEffects {
            tmux: false,
            notifications: false,
            auto_compact: false,
            removed_notification_groups: Some(removed.clone()),
            tmux_sync_calls: None,
        };

        let input: HookInput =
            serde_json::from_str(r#"{"session_id":"end-sess","cwd":"/tmp/test"}"#)
                .expect("valid JSON");

        process_hook_event_impl(HookEvent::SessionEnd, input, sessions_dir, &side_effects)
            .expect("hook should succeed");

        let recorded = removed.lock().expect("lock").clone();
        assert_eq!(recorded, expected_removed);
    }

    #[rstest]
    #[case::sweep_paused_keeps_one(SessionStatus::Paused, SessionStatus::Paused)]
    #[case::user_ended_clears(SessionStatus::Running, SessionStatus::Ended)]
    #[case::user_ctrlc_from_stopped_clears(SessionStatus::Stopped, SessionStatus::Ended)]
    fn session_end_syncs_pane_status_preserving_paused(
        #[case] initial_status: SessionStatus,
        #[case] expected_synced: SessionStatus,
    ) {
        // Sweep flips the status to Paused before SIGTERM, so a Paused
        // session reaching SessionEnd is the sweep path; that Paused must
        // be pushed through to tmux unchanged, not turned into Ended.
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let sessions_dir = temp_dir.path();

        let mut session = create_test_session(Some(TmuxInfo {
            session_name: "main".to_string(),
            window_name: "claude".to_string(),
            window_index: 0,
            pane_id: "%42".to_string(),
        }));
        session.session_id = "pane-sess".to_string();
        session.status = initial_status;
        store::save_session_to(sessions_dir, &session).expect("save");

        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let side_effects = SideEffects {
            tmux: false,
            notifications: false,
            auto_compact: false,
            removed_notification_groups: None,
            tmux_sync_calls: Some(calls.clone()),
        };

        let input: HookInput =
            serde_json::from_str(r#"{"session_id":"pane-sess","cwd":"/tmp/test"}"#)
                .expect("valid JSON");

        process_hook_event_impl(HookEvent::SessionEnd, input, sessions_dir, &side_effects)
            .expect("hook should succeed");

        let recorded = calls.lock().expect("lock").clone();
        assert_eq!(
            recorded,
            vec![(
                Some("%42".to_string()),
                Some(expected_synced),
                sessions_dir.to_path_buf(),
            )],
        );
    }

    /// Runs a PostToolUse hook with the given initial pending-id set and the
    /// given raw JSON payload, then returns the resulting pending set.
    fn run_post_tool_use(initial: &[&str], payload: &str) -> BTreeSet<String> {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let sessions_dir = temp_dir.path();

        let mut existing = create_test_session(None);
        existing.session_id = "bg-sess".to_string();
        existing.pending_bg_task_ids = initial.iter().map(|s| (*s).to_string()).collect();
        store::save_session_to(sessions_dir, &existing).expect("save");

        let input: HookInput = serde_json::from_str(payload).expect("valid JSON");
        process_hook_event_impl(
            HookEvent::PostToolUse,
            input,
            sessions_dir,
            &SideEffects::none(),
        )
        .expect("hook should succeed");

        store::load_session_from(sessions_dir, "bg-sess")
            .expect("load")
            .expect("session exists")
            .pending_bg_task_ids
    }

    fn set_of(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|s| (*s).to_string()).collect()
    }

    #[rstest]
    // Bash launch with backgroundTaskId inserts the id.
    #[case::inserts_bg_id(
        &[],
        r#"{"session_id":"bg-sess","cwd":"/tmp/test","tool_name":"Bash","tool_response":{"backgroundTaskId":"bg-123"}}"#,
        &["bg-123"],
    )]
    // Non-bg PostToolUse must leave the set alone (Read/Edit/etc. ship
    // object payloads without backgroundTaskId).
    #[case::read_does_not_change(
        &["bg-9"],
        r#"{"session_id":"bg-sess","cwd":"/tmp/test","tool_name":"Read","tool_response":{"file_path":"/tmp/x","content":"hi"}}"#,
        &["bg-9"],
    )]
    // tool_response shape varies per tool and Anthropic does not document a
    // schema (anthropics/claude-code#3671). Anything that is not an object
    // with backgroundTaskId must deserialize cleanly and leave the set alone.
    #[case::mcp_array_does_not_change(
        &["bg-9"],
        r#"{"session_id":"bg-sess","cwd":"/tmp/test","tool_response":[{"type":"text","text":"x"}]}"#,
        &["bg-9"],
    )]
    #[case::null_bg_id_does_not_insert(
        &[],
        r#"{"session_id":"bg-sess","cwd":"/tmp/test","tool_name":"Bash","tool_response":{"backgroundTaskId":null}}"#,
        &[],
    )]
    #[case::no_tool_response_does_not_change(
        &["bg-9"],
        r#"{"session_id":"bg-sess","cwd":"/tmp/test","tool_name":"Bash"}"#,
        &["bg-9"],
    )]
    // BashOutput with a terminal status removes the referenced shell id.
    #[case::bash_output_completed_removes(
        &["bg-1", "bg-2"],
        r#"{"session_id":"bg-sess","cwd":"/tmp/test","tool_name":"BashOutput","tool_input":{"shell_id":"bg-1"},"tool_response":{"status":"completed"}}"#,
        &["bg-2"],
    )]
    #[case::bash_output_killed_removes(
        &["bg-1"],
        r#"{"session_id":"bg-sess","cwd":"/tmp/test","tool_name":"BashOutput","tool_input":{"shell_id":"bg-1"},"tool_response":{"status":"killed"}}"#,
        &[],
    )]
    #[case::bash_output_failed_removes(
        &["bg-1"],
        r#"{"session_id":"bg-sess","cwd":"/tmp/test","tool_name":"BashOutput","tool_input":{"shell_id":"bg-1"},"tool_response":{"status":"failed"}}"#,
        &[],
    )]
    // BashOutput while the shell is still running must leave the set alone.
    #[case::bash_output_running_keeps(
        &["bg-1"],
        r#"{"session_id":"bg-sess","cwd":"/tmp/test","tool_name":"BashOutput","tool_input":{"shell_id":"bg-1"},"tool_response":{"status":"running"}}"#,
        &["bg-1"],
    )]
    // Legacy `bash_id` field name fallback (current builds use `shell_id`).
    #[case::bash_output_legacy_bash_id(
        &["bg-1"],
        r#"{"session_id":"bg-sess","cwd":"/tmp/test","tool_name":"BashOutput","tool_input":{"bash_id":"bg-1"},"tool_response":{"status":"completed"}}"#,
        &[],
    )]
    // KillShell removes the referenced id unconditionally — the tool's
    // entire purpose is to terminate the bg task.
    #[case::kill_shell_removes(
        &["bg-1", "bg-2"],
        r#"{"session_id":"bg-sess","cwd":"/tmp/test","tool_name":"KillShell","tool_input":{"shell_id":"bg-1"}}"#,
        &["bg-2"],
    )]
    // Schema mismatch: BashOutput with no parseable status — leave set
    // alone (a stale id is preferable to dropping a real bg task on the
    // floor; see call-site comment).
    #[case::bash_output_unknown_status_keeps(
        &["bg-1"],
        r#"{"session_id":"bg-sess","cwd":"/tmp/test","tool_name":"BashOutput","tool_input":{"shell_id":"bg-1"},"tool_response":{}}"#,
        &["bg-1"],
    )]
    fn post_tool_use_updates_pending_bg_tasks(
        #[case] initial: &[&str],
        #[case] payload: &str,
        #[case] expected: &[&str],
    ) {
        assert_eq!(run_post_tool_use(initial, payload), set_of(expected));
    }

    #[test]
    fn stop_does_not_clear_pending_bg_tasks() {
        // Synthetic Stop after a bg launch must leave the pending set
        // intact so that `sweep` can see it and skip auto-pause. Removal
        // only happens via BashOutput/KillShell PostToolUse.
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let sessions_dir = temp_dir.path();

        let mut existing = create_test_session(None);
        existing.session_id = "bg-stop".to_string();
        existing.pending_bg_task_ids.insert("bg-1".to_string());
        store::save_session_to(sessions_dir, &existing).expect("save");

        let side_effects = SideEffects {
            tmux: false,
            notifications: false,
            auto_compact: true,
            removed_notification_groups: None,
            tmux_sync_calls: None,
        };

        let input: HookInput =
            serde_json::from_str(r#"{"session_id":"bg-stop","cwd":"/tmp/test"}"#)
                .expect("valid JSON");

        process_hook_event_impl(HookEvent::Stop, input, sessions_dir, &side_effects)
            .expect("hook should succeed");

        let reloaded = store::load_session_from(sessions_dir, "bg-stop")
            .expect("load")
            .expect("session exists");
        assert_eq!(reloaded.pending_bg_task_ids, set_of(&["bg-1"]));
    }

    #[rstest]
    #[case::stop_resets_existing_read(
        HookEvent::Stop,
        None,
        SessionStatus::Stopped,
        Some(Utc::now()),
        None
    )]
    #[case::running_keeps_read(
        HookEvent::PreToolUse,
        None,
        SessionStatus::Stopped,
        Some(Utc::now()),
        Some(()),
    )]
    #[case::idle_prompt_resets_read(
        HookEvent::Notification,
        Some("idle_prompt"),
        SessionStatus::Stopped,
        Some(Utc::now()),
        None
    )]
    fn read_at_reset_on_stopped_transition(
        #[case] event: HookEvent,
        #[case] notification_type: Option<&str>,
        #[case] initial_status: SessionStatus,
        #[case] initial_read_at: Option<chrono::DateTime<Utc>>,
        #[case] expected_read_marker: Option<()>,
    ) {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let sessions_dir = temp_dir.path();

        let mut existing = create_test_session(None);
        existing.session_id = "read-sess".to_string();
        existing.status = initial_status;
        existing.read_at = initial_read_at;
        store::save_session_to(sessions_dir, &existing).expect("save");

        let payload = if let Some(nt) = notification_type {
            format!(
                r#"{{"session_id":"read-sess","cwd":"/tmp/test","notification_type":"{}"}}"#,
                nt
            )
        } else {
            r#"{"session_id":"read-sess","cwd":"/tmp/test"}"#.to_string()
        };
        let input: HookInput = serde_json::from_str(&payload).expect("valid JSON");

        process_hook_event_impl(event, input, sessions_dir, &SideEffects::none())
            .expect("hook should succeed");

        let reloaded = store::load_session_from(sessions_dir, "read-sess")
            .expect("load")
            .expect("session exists");
        assert_eq!(reloaded.read_at.is_some(), expected_read_marker.is_some());
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
            pending_bg_task_ids: BTreeSet::new(),
            read_at: None,
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

            let result = process_hook_event_impl(
                HookEvent::SessionStart,
                input,
                temp_dir.path(),
                &SideEffects::none(),
            )
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
            let startup_result = process_hook_event_impl(
                HookEvent::SessionStart,
                startup_input,
                temp_dir.path(),
                &SideEffects::none(),
            )
            .expect("startup should succeed");
            assert_eq!(
                startup_result,
                ProcessResult::Skipped,
                "startup event should be skipped to prevent empty session creation"
            );

            // Second event: "resume" with the actual session_id being restored
            let resume_input = create_test_input_with_source(None, Some("resume"));
            let resume_result = process_hook_event_impl(
                HookEvent::SessionStart,
                resume_input,
                temp_dir.path(),
                &SideEffects::none(),
            )
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

            let result = process_hook_event_impl(
                HookEvent::SessionStart,
                input,
                temp_dir.path(),
                &SideEffects::none(),
            )
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

            let result = process_hook_event_impl(
                HookEvent::UserPromptSubmit,
                input,
                temp_dir.path(),
                &SideEffects::none(),
            )
            .expect("should succeed");

            assert_eq!(
                result,
                ProcessResult::SessionSaved,
                "user-prompt-submit should create the session"
            );
        }

        #[test]
        fn new_session_full_flow_startup_then_user_prompt() {
            let temp_dir = create_temp_sessions_dir();

            // Simulate `claude` (without -c): SessionStart(startup) is skipped,
            // then UserPromptSubmit creates the session.
            // This also exercises the code path where UserPromptSubmit sets the
            // pane option (when running inside tmux).
            let startup_input = create_test_input_with_source(None, Some("startup"));
            let startup_result = process_hook_event_impl(
                HookEvent::SessionStart,
                startup_input,
                temp_dir.path(),
                &SideEffects::none(),
            )
            .expect("startup should succeed");
            assert_eq!(startup_result, ProcessResult::Skipped);

            // No session file should exist after startup skip
            let session =
                store::load_session_from(temp_dir.path(), "test-123").expect("load should succeed");
            assert!(
                session.is_none(),
                "session should not exist after skipped startup"
            );

            // UserPromptSubmit creates the session
            let prompt_input = create_test_input(None);
            let prompt_result = process_hook_event_impl(
                HookEvent::UserPromptSubmit,
                prompt_input,
                temp_dir.path(),
                &SideEffects::none(),
            )
            .expect("user-prompt-submit should succeed");
            assert_eq!(prompt_result, ProcessResult::SessionSaved);

            // Session should now exist
            let session = store::load_session_from(temp_dir.path(), "test-123")
                .expect("load should succeed")
                .expect("session should exist after user-prompt-submit");
            assert_eq!(session.session_id, "test-123");
            assert_eq!(session.status, SessionStatus::Running);
        }
    }

    mod evict_paused_on_pane_takeover_tests {
        use super::*;
        use chrono::Utc;
        use rstest::{fixture, rstest};
        use tempfile::TempDir;

        #[fixture]
        fn temp_dir() -> TempDir {
            TempDir::new().expect("temp dir creation should succeed")
        }

        fn make_paused_session(session_id: &str, pane_id: &str) -> Session {
            let now = Utc::now();
            Session {
                session_id: session_id.to_string(),
                cwd: std::path::PathBuf::from("/tmp/test"),
                transcript_path: None,
                tty: None,
                tmux_info: Some(TmuxInfo {
                    session_name: "main".into(),
                    window_name: "win".into(),
                    window_index: 0,
                    pane_id: pane_id.to_string(),
                }),
                status: SessionStatus::Paused,
                created_at: now,
                updated_at: now,
                last_message: None,
                current_tool: None,
                label: None,
                ancestor_session_ids: Vec::new(),
                pending_bg_task_ids: BTreeSet::new(),
                read_at: None,
            }
        }

        #[rstest]
        #[case::ends_on_pane_takeover(
            SessionStatus::Paused,
            "old",
            "%42",
            "%42",
            "new",
            SessionStatus::Ended
        )]
        #[case::keeps_when_session_id_matches(
            // `claude -c <same-id>` resume: session_id matches, must be left
            // as Paused so the running hook later transitions it back.
            SessionStatus::Paused,
            "same",
            "%42",
            "%42",
            "same",
            SessionStatus::Paused,
        )]
        #[case::keeps_when_pane_differs(
            SessionStatus::Paused,
            "other",
            "%99",
            "%42",
            "new",
            SessionStatus::Paused
        )]
        #[case::keeps_when_not_paused(
            // A Running session sharing the pane (shouldn't happen in practice
            // but guards against accidental termination of active sessions).
            SessionStatus::Running,
            "active",
            "%42",
            "%42",
            "new",
            SessionStatus::Running,
        )]
        fn evict_paused_sessions_on_pane_takeover_cases(
            temp_dir: TempDir,
            #[case] initial_status: SessionStatus,
            #[case] session_id: &str,
            #[case] session_pane: &str,
            #[case] takeover_pane: &str,
            #[case] takeover_session_id: &str,
            #[case] expected_status: SessionStatus,
        ) {
            let mut session = make_paused_session(session_id, session_pane);
            session.status = initial_status;
            store::save_session_to(temp_dir.path(), &session).expect("save");

            evict_paused_sessions_on_pane_takeover(
                temp_dir.path(),
                takeover_pane,
                takeover_session_id,
            );

            let reloaded = store::load_session_from(temp_dir.path(), session_id)
                .expect("load")
                .expect("session exists");
            assert_eq!(reloaded.status, expected_status);
        }

        #[rstest]
        fn missing_sessions_dir_is_noop(temp_dir: TempDir) {
            let missing = temp_dir.path().join("does-not-exist");
            evict_paused_sessions_on_pane_takeover(&missing, "%42", "new");
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
