//! Ctrl+g title generation from a session's transcript, triggered from
//! `AppMode::Edit` (see `title_edit`).

use indoc::formatdoc;

use crate::commands::cc::claude_sessions;

use super::app::{App, AppMode};

/// Everything a background worker needs to generate one session's title.
#[derive(Debug, PartialEq, Eq)]
pub struct GenerateTitleRequest {
    pub session_id: String,
    pub generation_id: u64,
    pub prompt: String,
}

/// Handles `Ctrl+g` from `AppMode::Edit`: reads the session's first user
/// message and last assistant message from its `.jsonl`, builds the LLM
/// prompt, and marks generation in flight. Returns `None` without spawning
/// anything when a generation is already in flight -- each press is a
/// billed LLM call, so repeats are ignored rather than piling up
/// subprocesses -- or (after setting an error) when neither message is
/// available, or when the app isn't in `AppMode::Edit` for a known session.
pub(super) fn request_generate_title(app: &mut App) -> Option<GenerateTitleRequest> {
    let session_id = match &app.mode {
        AppMode::Edit { session_id } => session_id.clone(),
        _ => return None,
    };
    if app.title_generating.is_some() {
        return None;
    }
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

    app.title_generation_seq += 1;
    let generation_id = app.title_generation_seq;
    app.title_generating = Some((session_id.clone(), generation_id));

    Some(GenerateTitleRequest {
        session_id,
        generation_id,
        prompt,
    })
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

/// Applies a background title-generation result. Drops it entirely if it
/// doesn't match the currently tracked in-flight request -- either a stale
/// result from a request that was superseded (cancelled and re-triggered
/// for the same session), or the user has since left edit mode (Esc /
/// Enter / different selection). `label` must never be touched from here.
pub(super) fn apply_title_generated(
    app: &mut App,
    session_id: &str,
    generation_id: u64,
    result: Result<String, String>,
) {
    let is_current_request =
        app.title_generating.as_ref() == Some(&(session_id.to_string(), generation_id));
    if !is_current_request {
        return;
    }
    app.title_generating = None;

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

        let request = request_generate_title(&mut app);

        assert_eq!(
            (request.is_none(), app.title_generating.clone()),
            (true, None)
        );
    }

    #[test]
    fn request_generate_title_sets_error_without_transcript() {
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
    fn request_generate_title_ignores_repeat_press_while_in_flight() {
        let mut app = App::with_sessions(vec![test_session("s1")]);
        app.enter_edit_title();
        app.title_generating = Some(("s1".to_string(), 1));

        let request = request_generate_title(&mut app);

        assert_eq!(
            (request.is_none(), app.title_generating.clone()),
            (true, Some(("s1".to_string(), 1)))
        );
    }

    #[test]
    fn apply_title_generated_updates_buffer_on_success_when_still_editing() {
        let mut app = App::with_sessions(vec![test_session("s1")]);
        app.enter_edit_title();
        app.title_generating = Some(("s1".to_string(), 1));

        apply_title_generated(&mut app, "s1", 1, Ok("New Title".to_string()));

        assert_eq!(
            (app.edit_title_query.clone(), app.title_generating.clone()),
            ("New Title".to_string(), None)
        );
    }

    #[test]
    fn apply_title_generated_sets_error_on_failure() {
        let mut app = App::with_sessions(vec![test_session("s1")]);
        app.enter_edit_title();
        app.title_generating = Some(("s1".to_string(), 1));

        apply_title_generated(&mut app, "s1", 1, Err("boom".to_string()));

        assert_eq!(
            (app.title_generating.clone(), app.error_message.clone()),
            (None, Some("Failed to generate title: boom".to_string()))
        );
    }

    #[test]
    fn apply_title_generated_is_noop_after_leaving_edit_mode() {
        let mut app = App::with_sessions(vec![test_session("s1")]);
        app.enter_edit_title();
        app.title_generating = Some(("s1".to_string(), 1));
        app.cancel_edit_title(); // back to Normal; also clears title_generating
        let buffer_before = app.edit_title_query.clone();

        apply_title_generated(&mut app, "s1", 1, Ok("Should not apply".to_string()));

        assert_eq!(app.edit_title_query, buffer_before);
    }

    #[test]
    fn apply_title_generated_ignores_stale_generation_id() {
        // A newer request (id 2) is in flight; a late result from an
        // earlier, superseded request (id 1) must not clobber it.
        let mut app = App::with_sessions(vec![test_session("s1")]);
        app.enter_edit_title();
        app.title_generating = Some(("s1".to_string(), 2));
        let buffer_before = app.edit_title_query.clone();

        apply_title_generated(&mut app, "s1", 1, Ok("Stale title".to_string()));

        assert_eq!(
            (app.edit_title_query.clone(), app.title_generating.clone()),
            (buffer_before, Some(("s1".to_string(), 2)))
        );
    }
}
