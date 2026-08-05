//! Ctrl+g title generation from a session's transcript, triggered from
//! `AppMode::Edit` (see `title_edit`).

use indoc::formatdoc;

use crate::commands::cc::claude_sessions;

use super::app::{App, AppMode};

/// Everything a background worker needs to generate one session's title.
#[derive(Debug, PartialEq, Eq)]
pub struct GenerateTitleRequest {
    pub session_id: String,
    pub prompt: String,
}

/// Handles `Ctrl+g` from `AppMode::Edit`: reads the session's first user
/// message and last assistant message from its `.jsonl`, builds the LLM
/// prompt, and marks generation in flight. Returns `None` (after setting an
/// error) when neither message is available -- there is nothing to
/// summarize -- or when the app isn't in `AppMode::Edit` for a known
/// session.
pub(super) fn request_generate_title(app: &mut App) -> Option<GenerateTitleRequest> {
    let session_id = match &app.mode {
        AppMode::Edit { session_id } => session_id.clone(),
        _ => return None,
    };
    let cwd = app
        .sessions
        .iter()
        .find(|s| s.session_id == session_id)?
        .cwd
        .clone();

    let first_user_message = claude_sessions::get_first_user_message(&cwd, &session_id);
    let last_assistant_message = claude_sessions::get_last_assistant_message(&cwd, &session_id);

    if first_user_message.is_none() && last_assistant_message.is_none() {
        app.set_error("No conversation content to generate a title from".to_string());
        return None;
    }

    let prompt = build_prompt(
        first_user_message.as_deref().unwrap_or(""),
        last_assistant_message.as_deref().unwrap_or(""),
    );

    app.title_generating = Some(session_id.clone());

    Some(GenerateTitleRequest { session_id, prompt })
}

/// Builds the LLM prompt for generating a session title from its transcript.
pub fn build_prompt(first_user_message: &str, last_assistant_message: &str) -> String {
    formatdoc! {r#"
        Task: Generate a short session title in Japanese describing what the user is working on in this Claude Code session.

        Requirements:
        - Japanese, 10-25 characters
        - Include concrete identifiers (repository, feature, command name) when available
        - No trailing punctuation
        - One line only

        <first-user-message>
        {first_user_message}
        </first-user-message>

        <latest-assistant-message>
        {last_assistant_message}
        </latest-assistant-message>

        IMPORTANT: Output ONLY the title. Do not explain."#}
}

/// Applies a background title-generation result. No-ops if the user has
/// since left edit mode for this session (Esc / Enter / different
/// selection) -- the result has nowhere to land, and `label` must never be
/// touched from here.
pub(super) fn apply_title_generated(
    app: &mut App,
    session_id: &str,
    result: Result<String, String>,
) {
    if app.title_generating.as_deref() == Some(session_id) {
        app.title_generating = None;
    }

    let is_editing_this_session = matches!(
        &app.mode,
        AppMode::Edit { session_id: editing_id } if editing_id == session_id
    );
    if !is_editing_this_session {
        return;
    }

    match result {
        Ok(title) => app.update_edit_title_query(title),
        Err(e) => app.set_error(format!("Failed to generate title: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::types::{Session, SessionStatus};
    use std::path::PathBuf;

    fn test_session(id: &str) -> Session {
        Session {
            session_id: id.to_string(),
            cwd: PathBuf::from("/tmp/test"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: std::collections::BTreeSet::new(),
            pending_agent_task_ids: std::collections::BTreeSet::new(),
            read_at: None,
            sweep_signaled: false,
        }
    }

    #[test]
    fn build_prompt_includes_both_messages() {
        let prompt = build_prompt("fix the login bug", "I've updated auth.rs");

        assert_eq!(
            prompt,
            formatdoc! {r#"
                Task: Generate a short session title in Japanese describing what the user is working on in this Claude Code session.

                Requirements:
                - Japanese, 10-25 characters
                - Include concrete identifiers (repository, feature, command name) when available
                - No trailing punctuation
                - One line only

                <first-user-message>
                fix the login bug
                </first-user-message>

                <latest-assistant-message>
                I've updated auth.rs
                </latest-assistant-message>

                IMPORTANT: Output ONLY the title. Do not explain."#}
        );
    }

    #[test]
    fn request_generate_title_is_noop_outside_edit_mode() {
        let mut app = App::with_sessions(vec![test_session("s1")]);
        // app.mode defaults to Normal

        let request = request_generate_title(&mut app);

        assert_eq!(
            (request.is_none(), app.title_generating.clone()),
            (true, None)
        );
    }

    #[test]
    fn request_generate_title_sets_error_without_transcript() {
        // No real .jsonl on disk for this session id -> both messages are None.
        let mut app = App::with_sessions(vec![test_session("s1")]);
        app.enter_edit_title();

        let request = request_generate_title(&mut app);

        assert_eq!(
            (
                request.is_none(),
                app.title_generating.clone(),
                app.error_message.clone()
            ),
            (
                true,
                None,
                Some("No conversation content to generate a title from".to_string())
            )
        );
    }

    #[test]
    fn apply_title_generated_updates_buffer_on_success_when_still_editing() {
        let mut app = App::with_sessions(vec![test_session("s1")]);
        app.enter_edit_title();
        app.title_generating = Some("s1".to_string());

        apply_title_generated(&mut app, "s1", Ok("New Title".to_string()));

        assert_eq!(
            (app.edit_title_query.clone(), app.title_generating.clone()),
            ("New Title".to_string(), None)
        );
    }

    #[test]
    fn apply_title_generated_sets_error_on_failure() {
        let mut app = App::with_sessions(vec![test_session("s1")]);
        app.enter_edit_title();
        app.title_generating = Some("s1".to_string());

        apply_title_generated(&mut app, "s1", Err("boom".to_string()));

        assert_eq!(
            (app.title_generating.clone(), app.error_message.clone()),
            (None, Some("Failed to generate title: boom".to_string()))
        );
    }

    #[test]
    fn apply_title_generated_is_noop_after_leaving_edit_mode() {
        let mut app = App::with_sessions(vec![test_session("s1")]);
        app.enter_edit_title();
        app.title_generating = Some("s1".to_string());
        app.cancel_edit_title(); // back to Normal
        // cancel_edit_title does not clear the buffer itself -- only the
        // mode -- so the no-op check below is against whatever value was
        // seeded on entry, not an empty string.
        let buffer_before = app.edit_title_query.clone();

        apply_title_generated(&mut app, "s1", Ok("Should not apply".to_string()));

        assert_eq!(app.edit_title_query, buffer_before);
    }
}
