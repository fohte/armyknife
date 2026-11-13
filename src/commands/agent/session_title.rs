use crate::commands::agent::claude_sessions;
use crate::commands::agent::types::Session;
use crate::commands::agent::{store, window_status};
use anyhow::Result;
use std::path::Path;

/// Saves a manual title and mirrors it to the session's tmux window when available.
pub(crate) fn set_session_title(session_id: &str, title: &str) -> Result<Option<String>> {
    set_session_title_in(&store::sessions_dir()?, session_id, title)
}

pub(crate) fn set_session_title_in(
    sessions_dir: &Path,
    session_id: &str,
    title: &str,
) -> Result<Option<String>> {
    let trimmed = title.trim();
    let label = (!trimmed.is_empty()).then(|| trimmed.to_string());
    store::update_session_label_in(sessions_dir, session_id, label.clone())?;

    if let Ok(Some(mut session)) = store::load_session_from(sessions_dir, session_id) {
        session.label = label.clone();
        window_status::sync_window_title_for_session(&session);
    }

    Ok(label)
}

pub(crate) fn session_title(session: &Session) -> Option<String> {
    session
        .label
        .as_deref()
        .map(claude_sessions::normalize_title)
        .or_else(|| claude_sessions::get_session_title(&session.cwd, &session.session_id))
}

pub(crate) fn title_display_name(session: &Session) -> String {
    session_title(session).unwrap_or_else(|| {
        let fallback = session
            .cwd
            .file_name()
            .and_then(|name| name.to_str())
            .map(String::from)
            .unwrap_or_else(|| session.cwd.display().to_string());
        claude_sessions::normalize_title(&fallback)
    })
}

pub(crate) fn title_display_name_or_dash(session: &Session) -> String {
    session_title(session).unwrap_or_else(|| "-".to_string())
}
