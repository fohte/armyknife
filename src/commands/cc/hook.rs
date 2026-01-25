use std::io::{self, Read};

use anyhow::Result;
use chrono::Utc;
use clap::Args;

use super::error::CcError;
use super::store;
use super::tmux;
use super::tty;
use super::types::{HookEvent, HookInput, Session, SessionStatus};

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
    let tmux_info = tty.as_ref().and_then(|t| tmux::get_tmux_info_from_tty(t));

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

    let input: HookInput = serde_json::from_str(&json_str)?;

    Ok(input)
}

/// Determines the session status based on the event and input.
/// Note: SessionEnd is handled separately in run() before this function is called.
fn determine_status(event: HookEvent, input: &HookInput) -> SessionStatus {
    match event {
        HookEvent::Stop => SessionStatus::Stopped,
        HookEvent::Notification => {
            // Check if it's a permission prompt notification
            if input
                .notification_type
                .as_ref()
                .is_some_and(|t| t == "permission_prompt")
            {
                SessionStatus::WaitingInput
            } else {
                SessionStatus::Running
            }
        }
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
}
