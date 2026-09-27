use crossterm::event::{KeyCode, KeyModifiers};

use super::KeyEffects;
use super::app::{App, AppMode};
use super::event::KeyEvent;

/// Handles opening the selected review after leaving the watch TUI.
pub(super) fn handle(app: &mut App, key: KeyEvent) -> Option<KeyEffects> {
    if app.mode != AppMode::Normal
        || (key.code, key.modifiers) != (KeyCode::Char('o'), KeyModifiers::NONE)
    {
        return None;
    }

    app.clear_error();
    let Some(session) = app.selected_session() else {
        return Some(KeyEffects::default());
    };
    if session.crit_urls.is_empty() {
        return Some(KeyEffects::default());
    }
    if session.tmux_info.is_none() {
        app.set_error("Crit review requires a tmux pane".to_string());
        return Some(KeyEffects::default());
    }

    let session_id = session.session_id.clone();
    app.quit();
    Some(KeyEffects {
        open_crit_session_id: Some(session_id),
        ..Default::default()
    })
}
