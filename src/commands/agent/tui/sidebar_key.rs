use crossterm::event::{KeyCode, KeyModifiers};

use super::KeyEffects;
use super::app::{App, NarrowScreen};
use super::event::KeyEvent;

pub(super) fn handle(app: &mut App, key: KeyEvent) -> Option<KeyEffects> {
    if app.narrow_layout
        && (app.narrow_screen != NarrowScreen::Sidebar
            || matches!(key.code, KeyCode::Tab)
            || (key.code == KeyCode::Char('b') && key.modifiers == KeyModifiers::CONTROL))
    {
        return None;
    }

    match (key.code, key.modifiers) {
        (KeyCode::Tab, _) => {
            app.clear_error();
            app.toggle_sidebar_focus();
            Some(KeyEffects::default())
        }
        (KeyCode::Char('b'), KeyModifiers::CONTROL) => {
            app.clear_error();
            app.toggle_sidebar_visibility();
            Some(KeyEffects::default())
        }
        (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) if app.sidebar_focused => {
            app.clear_error();
            app.move_sidebar_cursor(1);
            Some(KeyEffects::default())
        }
        (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) if app.sidebar_focused => {
            app.clear_error();
            app.move_sidebar_cursor(-1);
            Some(KeyEffects::default())
        }
        (KeyCode::Char('h'), KeyModifiers::NONE) | (KeyCode::Left, _) if app.sidebar_focused => {
            app.clear_error();
            app.move_sidebar_to_parent();
            Some(KeyEffects::default())
        }
        (KeyCode::Char('l'), KeyModifiers::NONE) | (KeyCode::Right, _) if app.sidebar_focused => {
            app.clear_error();
            app.move_sidebar_to_child();
            Some(KeyEffects::default())
        }
        (KeyCode::Enter, _) if app.sidebar_focused => {
            app.clear_error();
            app.select_sidebar_cursor();
            if app.narrow_layout {
                app.show_narrow_session_list_screen();
            }
            Some(KeyEffects::default())
        }
        (KeyCode::Char('r'), KeyModifiers::NONE) if app.sidebar_focused => {
            app.clear_error();
            Some(KeyEffects {
                request_tq_sidebar_fetch: true,
                ..Default::default()
            })
        }
        _ => None,
    }
}
