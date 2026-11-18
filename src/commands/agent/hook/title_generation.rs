use super::super::claude_sessions;
use super::super::tui::title_generate;
use super::super::types::{Engine, Session};

pub(super) fn prompt(session: &Session) -> Option<String> {
    let first_message =
        claude_sessions::get_first_user_message_raw(&session.cwd, &session.session_id)?;
    let existing_title = claude_sessions::get_last_title(&session.cwd, &session.session_id);
    if !should_generate(
        session.engine,
        session.label.as_deref(),
        existing_title.as_deref(),
        &first_message,
    ) || claude_sessions::has_transcript_title_entry(&session.cwd, &session.session_id)
    {
        return None;
    }

    let first_message = claude_sessions::clean_peer_message(&first_message);
    let last_assistant_message =
        claude_sessions::get_last_assistant_message(&session.cwd, &session.session_id)
            .unwrap_or_default();
    Some(title_generate::build_prompt(
        &first_message,
        &last_assistant_message,
    ))
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
