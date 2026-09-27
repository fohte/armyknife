use crossterm::event::{KeyCode, KeyModifiers};

use super::KeyEffects;
use super::app::{App, AppMode, View};
use super::event::KeyEvent;
use super::{crit_key, sidebar_key, title_edit, worktree_session_children};
use crate::commands::agent::resume;
use crate::commands::agent::types::SessionStatus;
use crate::infra::tmux;

/// Focuses on the selected session's tmux pane.
fn focus_selected_session(app: &mut App) {
    if let Some(session) = app.selected_session()
        && let Some(ref tmux_info) = session.tmux_info
        && let Err(e) = tmux::focus_pane(&tmux_info.pane_id)
    {
        app.set_error(format!("Failed to focus tmux pane: {e}"));
    }
}

fn resume_selected_session(app: &mut App) {
    let Some(session) = app.selected_session() else {
        return;
    };

    match resume::respawn_paused_session(session) {
        Ok(pane_id) => {
            if let Err(e) = tmux::focus_pane(&pane_id) {
                app.set_error(format!("Failed to focus pane: {e}"));
            }
        }
        Err(e) => app.set_error(e.to_string()),
    }
}

/// Handles key events in Search mode.
fn handle_search_key_event(app: &mut App, key: KeyEvent) {
    match (key.code, key.modifiers) {
        // Cancel search
        (KeyCode::Esc, _) => {
            app.cancel_search();
        }

        // Confirm search and focus on selected session
        (KeyCode::Enter, _) => {
            app.confirm_search();
            focus_selected_session(app);
        }

        // Navigation within filtered results (Ctrl+n/p or arrow keys only)
        (KeyCode::Char('n'), KeyModifiers::CONTROL) | (KeyCode::Down, _) => {
            app.select_next();
        }
        (KeyCode::Char('p'), KeyModifiers::CONTROL) | (KeyCode::Up, _) => {
            app.select_previous();
        }

        // Clear entire search query
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
            app.update_search_query(String::new());
        }

        // Delete last word
        (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
            let query = app.search_query.clone();
            let trimmed = query.trim_end();
            let new_query = if let Some(pos) = trimmed.rfind(char::is_whitespace) {
                trimmed[..=pos].to_string()
            } else {
                String::new()
            };
            app.update_search_query(new_query);
        }

        // Delete character
        (KeyCode::Backspace, _) => {
            let mut query = app.search_query.clone();
            query.pop();
            app.update_search_query(query);
        }

        // Add character to search query (including j/k)
        (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            let mut query = app.search_query.clone();
            query.push(c);
            app.update_search_query(query);
        }

        _ => {}
    }
}

/// Handles key events in Normal mode.
fn handle_normal_key_event(app: &mut App, key: KeyEvent) {
    // Clear error message on any key press
    app.clear_error();

    match (key.code, key.modifiers) {
        // Enter search mode
        (KeyCode::Char('/'), _) => {
            app.enter_search_mode();
        }

        // Clear filter or quit
        (KeyCode::Esc, _) => {
            if app.narrow_layout && !app.sidebar_focused {
                if app.has_non_sidebar_filter() {
                    app.clear_non_sidebar_filters();
                } else {
                    app.show_narrow_sidebar_screen();
                }
            } else if app.has_filter() {
                app.clear_filter();
            } else {
                app.quit();
            }
        }

        // Quit
        (KeyCode::Char('q'), KeyModifiers::NONE) => {
            app.quit();
        }

        // Toggle the full key-binding list in the help bar
        (KeyCode::Char('?'), KeyModifiers::NONE) => {
            app.toggle_help();
        }

        // Navigation
        (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => {
            app.select_next();
        }
        (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => {
            app.select_previous();
        }

        // Move to the selected session's parent
        (KeyCode::Char('h'), KeyModifiers::NONE) | (KeyCode::Left, _) => {
            app.select_parent();
        }

        (KeyCode::Char('l'), KeyModifiers::NONE) | (KeyCode::Right, _) => {
            app.enter_drilldown();
        }

        // Focus on selected session's tmux pane
        (KeyCode::Enter, _) | (KeyCode::Char('f'), KeyModifiers::NONE) => {
            focus_selected_session(app);
        }

        // Resume a paused session
        (KeyCode::Char('r'), KeyModifiers::NONE) => {
            resume_selected_session(app);
        }

        // Delete selected session (with confirmation)
        (KeyCode::Char('d'), KeyModifiers::NONE) => {
            app.request_delete();
        }

        // Rename the selected session's title
        (KeyCode::Char('e'), KeyModifiers::NONE) => {
            app.enter_edit_title();
        }

        // Status filters (toggle). Use Ctrl-prefixed bindings so that plain
        // letters (`r`, `s`, `w`) remain available for other actions such as
        // resuming a paused session.
        (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
            app.toggle_status_filter(SessionStatus::WaitingInput);
        }
        (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
            app.toggle_status_filter(SessionStatus::Stopped);
        }
        (KeyCode::Char('r'), KeyModifiers::CONTROL) => {
            app.toggle_status_filter(SessionStatus::Running);
        }
        (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
            app.toggle_status_filter(SessionStatus::Paused);
        }

        // Quick select (1-9)
        (KeyCode::Char(c), KeyModifiers::NONE) if c.is_ascii_digit() && c != '0' => {
            let num = c.to_digit(10).unwrap_or(0) as usize;
            app.select_by_number(num);
        }

        _ => {}
    }
}

/// Handles key events in Confirm mode.
fn handle_confirm_key_event(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('y') => {
            if let Err(e) = app.confirm_delete() {
                app.set_error(format!("Failed to delete session: {e}"));
            }
        }
        KeyCode::Char('n') | KeyCode::Esc => {
            app.cancel_confirm();
        }
        _ => {}
    }
}

/// Handles key events based on the current view and sub-mode.
pub(super) fn handle_key_event(app: &mut App, key: KeyEvent) -> KeyEffects {
    // One-shot banners: any key press dismisses them so they do not
    // linger over later renders.
    if app.clean_progress.as_ref().is_some_and(|p| p.done) {
        app.clear_clean_progress();
    }

    match app.view {
        View::Session => handle_session_view_key_event(app, key),
        View::Clean => handle_clean_view_key_event(app, key),
    }
}

fn handle_session_view_key_event(app: &mut App, key: KeyEvent) -> KeyEffects {
    if app.mode == AppMode::Normal && app.narrow_layout && app.sidebar_focused {
        return handle_narrow_sidebar_key_event(app, key);
    }

    if app.mode == AppMode::Normal
        && let Some(effects) = sidebar_key::handle(app, key)
    {
        return effects;
    }

    // `c` from Normal mode enters the clean view from the session list.
    if app.mode == AppMode::Normal
        && let (KeyCode::Char('c'), KeyModifiers::NONE) = (key.code, key.modifiers)
    {
        let seeded = app.enter_clean_view();
        return KeyEffects {
            request_clean_pr_fetch: seeded,
            ..Default::default()
        };
    }
    // `p` previews the selected session's JSONL. No-op with no selection
    // or a session that has never emitted a transcript (JSONL is what
    // the viewer reads — nothing to open without it).
    if app.mode == AppMode::Normal
        && let (KeyCode::Char('p'), KeyModifiers::NONE) = (key.code, key.modifiers)
    {
        app.clear_error();
        let path = app
            .selected_session()
            .and_then(|s| s.transcript_path.clone());
        return KeyEffects {
            preview_session_path: path,
            ..Default::default()
        };
    }
    // `t` opens the selected session's linked tq task in the browser.
    // No-op with no selection or a session with no linked task.
    if app.mode == AppMode::Normal
        && let (KeyCode::Char('t'), KeyModifiers::NONE) = (key.code, key.modifiers)
    {
        app.clear_error();
        let selected_session_id = app.selected_session().map(|s| s.session_id.clone());
        let task_id = selected_session_id
            .as_deref()
            .and_then(|id| app.task_by_session.get(id))
            .map(|t| t.task_id.clone());
        return KeyEffects {
            fetch_task_url: task_id,
            ..Default::default()
        };
    }
    if let Some(effects) = crit_key::handle(app, key) {
        return effects;
    }

    match app.mode {
        AppMode::Normal => {
            handle_normal_key_event(app, key);
            KeyEffects::default()
        }
        AppMode::Search => {
            handle_search_key_event(app, key);
            KeyEffects::default()
        }
        AppMode::Confirm { .. } => {
            handle_confirm_key_event(app, key);
            KeyEffects::default()
        }
        AppMode::Edit { .. } => title_edit::handle_key_event(app, key),
    }
}

fn handle_narrow_sidebar_key_event(app: &mut App, key: KeyEvent) -> KeyEffects {
    if let Some(effects) = sidebar_key::handle(app, key) {
        return effects;
    }

    app.clear_error();
    match (key.code, key.modifiers) {
        (KeyCode::Char('q'), KeyModifiers::NONE) => app.quit(),
        (KeyCode::Char('?'), KeyModifiers::NONE) => app.toggle_help(),
        (KeyCode::Esc, _) if app.has_filter() => app.clear_filter(),
        (KeyCode::Esc, _) => app.quit(),
        _ => {}
    }
    KeyEffects::default()
}

/// Handles key events in the clean view.
fn handle_clean_view_key_event(app: &mut App, key: KeyEvent) -> KeyEffects {
    app.clear_error();
    match (key.code, key.modifiers) {
        // Cancel: return to the session list without acting.
        (KeyCode::Esc, _)
        | (KeyCode::Char('n'), KeyModifiers::NONE)
        | (KeyCode::Char('q'), KeyModifiers::NONE) => {
            app.exit_clean_view();
        }
        // Confirm: spawn detached child with all To-delete paths and
        // return to the session list so progress can show in its bottom bar.
        (KeyCode::Char('y'), KeyModifiers::NONE) => {
            let paths = app.clean_view.to_delete_paths();
            if paths.is_empty() {
                // Nothing to do — quietly fall back.
                app.exit_clean_view();
                return KeyEffects::default();
            }
            app.exit_clean_view();
            return KeyEffects {
                spawn_detached_clean: Some(paths),
                ..Default::default()
            };
        }
        (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => {
            app.clean_view.select_next();
        }
        (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => {
            app.clean_view.select_previous();
        }
        (KeyCode::Enter, _) => {
            if let Some(child) = app.clean_view.selected_session_child() {
                focus_session_child(app, &child);
            } else {
                app.clean_view.toggle_selected_section();
            }
        }
        _ => {}
    }
    KeyEffects::default()
}

fn focus_session_child(app: &mut App, child: &worktree_session_children::SessionChild) {
    let Some(pane_id) = child.pane_id.as_deref() else {
        app.set_error("No tmux pane for this session".to_string());
        return;
    };
    if let Err(e) = tmux::focus_pane(pane_id) {
        app.set_error(format!("Failed to focus tmux pane: {e}"));
    }
}
