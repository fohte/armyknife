use crate::commands::cc::store;
use crate::commands::cc::types::Session;
use anyhow::Result;
use ratatui::widgets::ListState;

/// Application state for the TUI.
pub struct App {
    /// All sessions loaded from disk.
    pub sessions: Vec<Session>,
    /// State for the list widget (tracks selected index).
    pub list_state: ListState,
    /// Whether the application should quit.
    pub should_quit: bool,
}

impl App {
    /// Creates a new App instance with initial session data.
    pub fn new() -> Result<Self> {
        let sessions = load_sessions()?;
        let mut list_state = ListState::default();

        // Select first item if there are any sessions
        if !sessions.is_empty() {
            list_state.select(Some(0));
        }

        Ok(Self {
            sessions,
            list_state,
            should_quit: false,
        })
    }

    /// Reloads sessions from disk.
    pub fn reload_sessions(&mut self) -> Result<()> {
        self.sessions = load_sessions()?;

        // Adjust selection if needed
        if self.sessions.is_empty() {
            self.list_state.select(None);
        } else if let Some(selected) = self.list_state.selected() {
            if selected >= self.sessions.len() {
                self.list_state.select(Some(self.sessions.len() - 1));
            }
        } else {
            self.list_state.select(Some(0));
        }

        Ok(())
    }

    /// Moves selection to the next item.
    pub fn select_next(&mut self) {
        if self.sessions.is_empty() {
            return;
        }

        let i = match self.list_state.selected() {
            Some(i) => {
                if i >= self.sessions.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    /// Moves selection to the previous item.
    pub fn select_previous(&mut self) {
        if self.sessions.is_empty() {
            return;
        }

        let i = match self.list_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.sessions.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    /// Selects a session by its 1-indexed number (1-9).
    pub fn select_by_number(&mut self, num: usize) {
        if num > 0 && num <= self.sessions.len() {
            self.list_state.select(Some(num - 1));
        }
    }

    /// Signals that the application should quit.
    pub fn quit(&mut self) {
        self.should_quit = true;
    }
}

/// Loads sessions from disk with cleanup.
fn load_sessions() -> Result<Vec<Session>> {
    store::cleanup_stale_sessions()?;
    store::list_sessions()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::types::SessionStatus;
    use chrono::Utc;
    use std::path::PathBuf;

    fn create_test_session(id: &str) -> Session {
        Session {
            session_id: id.to_string(),
            cwd: PathBuf::from("/tmp/test"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
        }
    }

    #[test]
    fn test_select_next_empty() {
        let mut app = App {
            sessions: vec![],
            list_state: ListState::default(),
            should_quit: false,
        };

        app.select_next();
        assert!(app.list_state.selected().is_none());
    }

    #[test]
    fn test_select_next_wraps() {
        let mut app = App {
            sessions: vec![create_test_session("1"), create_test_session("2")],
            list_state: ListState::default(),
            should_quit: false,
        };
        app.list_state.select(Some(1));

        app.select_next();
        assert_eq!(app.list_state.selected(), Some(0));
    }

    #[test]
    fn test_select_previous_wraps() {
        let mut app = App {
            sessions: vec![create_test_session("1"), create_test_session("2")],
            list_state: ListState::default(),
            should_quit: false,
        };
        app.list_state.select(Some(0));

        app.select_previous();
        assert_eq!(app.list_state.selected(), Some(1));
    }

    #[test]
    fn test_select_by_number() {
        let mut app = App {
            sessions: vec![
                create_test_session("1"),
                create_test_session("2"),
                create_test_session("3"),
            ],
            list_state: ListState::default(),
            should_quit: false,
        };
        app.list_state.select(Some(0));

        app.select_by_number(2);
        assert_eq!(app.list_state.selected(), Some(1));

        // Out of range should not change selection
        app.select_by_number(10);
        assert_eq!(app.list_state.selected(), Some(1));

        // Zero should not change selection
        app.select_by_number(0);
        assert_eq!(app.list_state.selected(), Some(1));
    }

    #[test]
    fn test_quit() {
        let mut app = App {
            sessions: vec![],
            list_state: ListState::default(),
            should_quit: false,
        };

        assert!(!app.should_quit);
        app.quit();
        assert!(app.should_quit);
    }
}
