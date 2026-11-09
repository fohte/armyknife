use crate::commands::agent::claude_sessions;
use crate::commands::agent::types::Session;

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
