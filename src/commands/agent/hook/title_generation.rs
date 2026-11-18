use std::path::Path;

use anyhow::Result;

use super::super::claude_sessions;
use super::super::store;
use super::super::tui::title_generate;
use super::super::types::{Engine, Session};
use super::side_effects::SideEffects;

pub(super) fn maybe_generate(session: &Session, sessions_dir: &Path, side_effects: &SideEffects) {
    if !side_effects.title_generation {
        return;
    }

    match store::has_auto_title_generation_attempt_from(sessions_dir, &session.session_id) {
        Ok(true) => return,
        Ok(false) => {}
        Err(error) => {
            tracing::warn!(
                event = "agent.generate_title.claim_check_failed",
                session = %session.session_id,
                error = %error,
            );
            return;
        }
    }

    let prompt = match prompt(session) {
        Ok(Some(prompt)) => prompt,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(
                event = "agent.generate_title.transcript_read_failed",
                session = %session.session_id,
                error = %error,
            );
            return;
        }
    };

    match store::claim_auto_title_generation_from(sessions_dir, &session.session_id) {
        Ok(true) => {
            if !side_effects.generate_title(&session.session_id, &prompt, session.label.clone())
                && let Err(error) = store::release_auto_title_generation_claim_from(
                    sessions_dir,
                    &session.session_id,
                )
            {
                tracing::warn!(
                    event = "agent.generate_title.claim_release_failed",
                    session = %session.session_id,
                    error = %error,
                );
            }
        }
        Ok(false) => {}
        Err(error) => tracing::warn!(
            event = "agent.generate_title.claim_failed",
            session = %session.session_id,
            error = %error,
        ),
    }
}

fn prompt(session: &Session) -> Result<Option<String>> {
    let Some(first_message) =
        claude_sessions::get_first_user_message_raw(&session.cwd, &session.session_id)
    else {
        return Ok(None);
    };
    let existing_title = claude_sessions::get_last_title(&session.cwd, &session.session_id);
    if !should_generate(
        session.engine,
        session.label.as_deref(),
        existing_title.as_deref(),
        &first_message,
    ) {
        return Ok(None);
    }
    if claude_sessions::has_transcript_title_entry(&session.cwd, &session.session_id)? {
        return Ok(None);
    }

    let first_message = claude_sessions::clean_peer_message(&first_message);
    let last_assistant_message =
        claude_sessions::get_last_assistant_message(&session.cwd, &session.session_id)
            .unwrap_or_default();
    Ok(Some(title_generate::build_prompt(
        &first_message,
        &last_assistant_message,
    )))
}

fn should_generate(
    engine: Engine,
    label: Option<&str>,
    existing_title: Option<&str>,
    first_message: &str,
) -> bool {
    engine == Engine::Claude
        && !label.is_some_and(|label| !label.trim().is_empty())
        && existing_title.is_none()
        && super::super::peer_message::is_peer_message(Some(first_message))
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::should_generate;
    use crate::commands::agent::types::Engine;

    #[rstest]
    #[case::peer_without_title(
        Engine::Claude,
        None,
        None,
        "Another peer session sent a message:\nInspect the sample report.",
        true
    )]
    #[case::existing_label(
        Engine::Claude,
        Some("Manual label"),
        None,
        "Another peer session sent a message:\nInspect the sample report.",
        false
    )]
    #[case::existing_transcript_title(
        Engine::Claude,
        None,
        Some("Transcript title"),
        "Another peer session sent a message:\nInspect the sample report.",
        false
    )]
    #[case::ordinary_first_prompt(Engine::Claude, None, None, "Inspect the sample report.", false)]
    #[case::codex_session(
        Engine::Codex,
        None,
        None,
        "Another peer session sent a message:\nInspect the sample report.",
        false
    )]
    fn automatic_title_requires_an_untitled_claude_peer_session(
        #[case] engine: Engine,
        #[case] label: Option<&str>,
        #[case] existing_title: Option<&str>,
        #[case] first_message: &str,
        #[case] expected: bool,
    ) {
        assert_eq!(
            should_generate(engine, label, existing_title, first_message),
            expected,
        );
    }
}
