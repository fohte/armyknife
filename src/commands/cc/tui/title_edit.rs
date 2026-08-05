//! Rename-in-place editing of a session's title (`Session.label`), entered
//! via the `e` key from `AppMode::Normal`. See `AppMode::Edit` for the mode
//! this drives.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyModifiers};

use crate::commands::cc::{store, window_status};
use crate::infra::tmux;

use super::app::{App, AppMode};
use super::event::KeyEvent;

impl App {
    /// Enters title-edit mode for the currently selected session. No-op
    /// without a selection. Seeds the edit buffer with the session's
    /// currently displayed title (falling back to an empty buffer in the
    /// rare case the title cache has not been built yet) so the user edits
    /// an existing title rather than retyping it from scratch.
    pub fn enter_edit_title(&mut self) {
        let Some(session_id) = self.selected_session().map(|s| s.session_id.clone()) else {
            return;
        };
        self.edit_title_query = self
            .get_cached_title(&session_id)
            .map(str::to_string)
            .unwrap_or_default();
        self.mode = AppMode::Edit { session_id };
        self.title_generating = None;
    }

    /// Replaces the edit buffer wholesale. Callers build the new value
    /// (append/backspace) and pass it in, mirroring `update_search_query`.
    pub fn update_edit_title_query(&mut self, query: String) {
        self.edit_title_query = query;
    }

    /// Leaves title-edit mode without persisting the buffer.
    pub fn cancel_edit_title(&mut self) {
        self.mode = AppMode::Normal;
        self.title_generating = None;
    }

    /// Confirms the title edit.
    ///
    /// An empty or whitespace-only buffer normalizes to `label = None`
    /// (reverting to the automatic title) rather than storing an empty
    /// string. On success, the label is reflected immediately in the
    /// in-memory session list and title cache (see `set_session_label`) and
    /// best-effort mirrored into the session's tmux window title option --
    /// a missing or dead pane silently skips that step without affecting
    /// the store write or the in-memory reflection. Only the store write's
    /// failure is surfaced to the caller.
    pub fn confirm_edit_title(&mut self) -> Result<()> {
        let session_id = match &self.mode {
            AppMode::Edit { session_id } => session_id.clone(),
            _ => return Ok(()),
        };

        let trimmed = self.edit_title_query.trim();
        let label = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };

        store::update_session_label(&session_id, label.clone())?;

        self.set_session_label(&session_id, label);
        self.sync_tmux_window_title(&session_id);
        self.mode = AppMode::Normal;
        self.title_generating = None;

        Ok(())
    }

    /// Best-effort push of the tmux window title option after a label
    /// change. Silently does nothing without a live tmux pane (session ran
    /// outside tmux, or its pane has since died) or if the sessions
    /// directory cannot be resolved -- this step is not allowed to fail the
    /// confirm.
    fn sync_tmux_window_title(&self, session_id: &str) {
        let Some(pane_id) = self
            .sessions
            .iter()
            .find(|s| s.session_id == session_id)
            .and_then(|s| s.tmux_info.as_ref())
            .map(|info| info.pane_id.as_str())
        else {
            return;
        };
        let Some(window_id) = tmux::get_window_id_for_pane(pane_id) else {
            return;
        };
        let Ok(sessions_dir) = store::sessions_dir() else {
            return;
        };
        let _ = window_status::sync_window_option(&window_id, &sessions_dir);
    }
}

/// Handles key events in `AppMode::Edit`. Mirrors the search mode's text
/// vocabulary (Esc/Enter/Backspace/char-append); a single-line title editor
/// needs no in-mode navigation (Ctrl+u/w, arrows).
pub(super) fn handle_key_event(app: &mut App, key: KeyEvent) -> super::KeyEffects {
    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) => {
            app.cancel_edit_title();
        }
        (KeyCode::Enter, _) => {
            if let Err(e) = app.confirm_edit_title() {
                app.set_error(format!("Failed to update label: {e}"));
            }
        }
        (KeyCode::Backspace, _) => {
            let mut query = app.edit_title_query.clone();
            query.pop();
            app.update_edit_title_query(query);
        }
        (KeyCode::Char('g'), KeyModifiers::CONTROL) => {
            if let Some(request) = super::title_generate::request_generate_title(app) {
                return super::KeyEffects {
                    generate_title_request: Some(request),
                    ..Default::default()
                };
            }
        }
        (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            let mut query = app.edit_title_query.clone();
            query.push(c);
            app.update_edit_title_query(query);
        }
        _ => {}
    }
    super::KeyEffects::default()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rstest::rstest;
    use tempfile::TempDir;

    use super::*;
    use crate::commands::cc::types::{Session, SessionStatus};

    fn create_test_session(id: &str) -> Session {
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

    fn session_label(app: &App, session_id: &str) -> Option<String> {
        app.sessions
            .iter()
            .find(|s| s.session_id == session_id)
            .and_then(|s| s.label.clone())
    }

    /// A temp cache root laid out so that, once `XDG_CACHE_HOME` is pointed
    /// at it via `temp_env`, `store::sessions_dir()` resolves inside it --
    /// letting `confirm_edit_title`'s real `store::update_session_label`
    /// call round-trip through a real (but disposable) session file instead
    /// of the developer's actual `~/.cache`.
    struct TempCacheRoot {
        #[expect(dead_code, reason = "kept alive to prevent cleanup until dropped")]
        temp_dir: TempDir,
        cache_home: String,
        sessions_dir: PathBuf,
    }

    fn temp_cache_root() -> TempCacheRoot {
        let temp_dir = TempDir::new().expect("temp dir creation should succeed");
        let cache_home = temp_dir.path().to_str().expect("utf8 path").to_string();
        let sessions_dir = temp_dir
            .path()
            .join("armyknife")
            .join("cc")
            .join("sessions");
        TempCacheRoot {
            temp_dir,
            cache_home,
            sessions_dir,
        }
    }

    #[rstest]
    fn enter_seeds_buffer_from_displayed_title() {
        let mut session = create_test_session("s1");
        session.label = Some("Existing Title".to_string());
        let mut app = App::with_sessions(vec![session]);

        app.enter_edit_title();

        assert_eq!(
            (app.mode.clone(), app.edit_title_query.clone()),
            (
                AppMode::Edit {
                    session_id: "s1".to_string()
                },
                "Existing Title".to_string(),
            )
        );
    }

    #[rstest]
    fn enter_is_noop_without_selection() {
        let mut app = App::with_sessions(vec![]);

        app.enter_edit_title();

        assert_eq!(
            (app.mode.clone(), app.edit_title_query.clone()),
            (AppMode::Normal, String::new())
        );
    }

    #[rstest]
    fn typing_appends_and_backspace_removes() {
        let mut app = App::with_sessions(vec![create_test_session("s1")]);
        app.enter_edit_title();
        // Start from an empty buffer regardless of what the session's
        // fallback title seeded -- this test only covers the append/delete
        // key vocabulary, seeding itself is covered separately.
        app.update_edit_title_query(String::new());

        for c in ['a', 'b'] {
            handle_key_event(
                &mut app,
                KeyEvent {
                    code: KeyCode::Char(c),
                    modifiers: KeyModifiers::NONE,
                },
            );
        }
        assert_eq!(app.edit_title_query, "ab");

        handle_key_event(
            &mut app,
            KeyEvent {
                code: KeyCode::Backspace,
                modifiers: KeyModifiers::NONE,
            },
        );
        assert_eq!(app.edit_title_query, "a");
    }

    #[rstest]
    fn ctrl_g_without_transcript_sets_error_and_returns_no_effects() {
        let session = create_test_session("s1");
        let mut app = App::with_sessions(vec![session]);
        app.enter_edit_title();

        let effects = handle_key_event(
            &mut app,
            KeyEvent {
                code: KeyCode::Char('g'),
                modifiers: KeyModifiers::CONTROL,
            },
        );

        assert_eq!(
            (effects, app.error_message.is_some()),
            (super::super::KeyEffects::default(), true)
        );
    }

    #[rstest]
    fn esc_cancels_without_persisting() {
        let session = create_test_session("s1");
        let mut app = App::with_sessions(vec![session]);
        app.enter_edit_title();
        app.update_edit_title_query("Should not be saved".to_string());

        handle_key_event(
            &mut app,
            KeyEvent {
                code: KeyCode::Esc,
                modifiers: KeyModifiers::NONE,
            },
        );

        assert_eq!(
            (app.mode.clone(), session_label(&app, "s1")),
            (AppMode::Normal, None)
        );
    }

    #[rstest]
    fn confirm_with_non_empty_buffer_persists_label_and_updates_cache() {
        let root = temp_cache_root();
        let session = create_test_session("s1");
        store::save_session_to(&root.sessions_dir, &session).expect("save should succeed");

        let mut app = App::with_sessions(vec![session]);
        app.enter_edit_title();
        app.update_edit_title_query("New Title".to_string());

        temp_env::with_vars([("XDG_CACHE_HOME", Some(root.cache_home.as_str()))], || {
            app.confirm_edit_title().expect("confirm should succeed");
        });

        assert_eq!(
            (
                app.mode.clone(),
                session_label(&app, "s1"),
                app.get_cached_title("s1").map(str::to_string),
            ),
            (
                AppMode::Normal,
                Some("New Title".to_string()),
                Some("New Title".to_string()),
            )
        );

        let reloaded = store::load_session_from(&root.sessions_dir, "s1")
            .expect("load should succeed")
            .expect("session exists");
        assert_eq!(reloaded.label, Some("New Title".to_string()));
    }

    #[rstest]
    fn confirm_with_empty_buffer_persists_none() {
        let root = temp_cache_root();
        let mut session = create_test_session("s1");
        session.label = Some("Old Title".to_string());
        store::save_session_to(&root.sessions_dir, &session).expect("save should succeed");

        let mut app = App::with_sessions(vec![session]);
        app.enter_edit_title();
        app.update_edit_title_query("   ".to_string());

        temp_env::with_vars([("XDG_CACHE_HOME", Some(root.cache_home.as_str()))], || {
            app.confirm_edit_title().expect("confirm should succeed");
        });

        assert_eq!(session_label(&app, "s1"), None);

        let reloaded = store::load_session_from(&root.sessions_dir, "s1")
            .expect("load should succeed")
            .expect("session exists");
        assert_eq!(reloaded.label, None);
    }
}
