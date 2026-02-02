use crate::commands::cc::store;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use notify::{
    EventKind, RecommendedWatcher, RecursiveMode, Watcher,
    event::{CreateKind, ModifyKind, RemoveKind},
};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

/// Key event with code and modifiers.
#[derive(Debug, Clone, Copy)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

/// Represents a change to a specific session file.
#[derive(Debug, Clone)]
pub struct SessionChange {
    pub session_id: String,
    pub change_type: SessionChangeType,
}

/// Type of change that occurred to a session file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionChangeType {
    Created,
    Modified,
    Deleted,
}

/// Events that can occur in the TUI.
pub enum AppEvent {
    /// A key was pressed.
    Key(KeyEvent),
    /// Session data changed on disk.
    /// Contains session changes if specific files were identified, or None for full reload.
    SessionsChanged(Option<Vec<SessionChange>>),
    /// Tick for periodic updates.
    Tick,
}

/// Event handler that combines keyboard input and file system events.
pub struct EventHandler {
    receiver: Receiver<AppEvent>,
    /// Watcher must be kept alive to receive events.
    _watcher: Option<RecommendedWatcher>,
}

impl EventHandler {
    /// Creates a new event handler.
    /// Spawns background threads for keyboard and file system monitoring.
    pub fn new() -> Result<Self> {
        let (tx, rx) = mpsc::channel();

        // Spawn keyboard event thread
        let key_tx = tx.clone();
        thread::spawn(move || {
            handle_keyboard_events(key_tx);
        });

        // Spawn tick thread for periodic updates
        let tick_tx = tx.clone();
        thread::spawn(move || {
            handle_tick_events(tick_tx);
        });

        // Set up file system watcher
        let watcher = setup_file_watcher(tx)?;

        Ok(Self {
            receiver: rx,
            _watcher: watcher,
        })
    }

    /// Receives the next event, blocking until one is available.
    pub fn next(&self) -> Result<AppEvent> {
        self.receiver
            .recv()
            .map_err(|e| anyhow::anyhow!("Event channel closed: {}", e))
    }
}

/// Handles keyboard events in a background thread.
fn handle_keyboard_events(tx: Sender<AppEvent>) {
    loop {
        // Poll for events with a short timeout
        if event::poll(Duration::from_millis(50)).unwrap_or(false)
            && let Ok(Event::Key(key)) = event::read()
            && key.kind == KeyEventKind::Press
        {
            let key_event = KeyEvent {
                code: key.code,
                modifiers: key.modifiers,
            };
            if tx.send(AppEvent::Key(key_event)).is_err() {
                // Channel closed, exit thread
                break;
            }
        }
    }
}

/// Handles tick events for periodic UI updates.
fn handle_tick_events(tx: Sender<AppEvent>) {
    loop {
        thread::sleep(Duration::from_secs(1));
        if tx.send(AppEvent::Tick).is_err() {
            // Channel closed, exit thread
            break;
        }
    }
}

/// Sets up a file system watcher for the sessions directory.
fn setup_file_watcher(tx: Sender<AppEvent>) -> Result<Option<RecommendedWatcher>> {
    let sessions_dir = match store::sessions_dir() {
        Ok(dir) => dir,
        Err(_) => return Ok(None),
    };

    // Create sessions directory if it doesn't exist
    if !sessions_dir.exists() {
        std::fs::create_dir_all(&sessions_dir)?;
    }

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            let changes = extract_session_changes(&event);
            if !changes.is_empty() {
                let _ = tx.send(AppEvent::SessionsChanged(Some(changes)));
            }
        }
    })?;

    watcher.watch(Path::new(&sessions_dir), RecursiveMode::NonRecursive)?;

    Ok(Some(watcher))
}

/// Extracts session IDs from file paths in the event.
fn extract_session_changes(event: &notify::Event) -> Vec<SessionChange> {
    let change_type = match event.kind {
        EventKind::Create(CreateKind::File) => SessionChangeType::Created,
        EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Name(_)) => SessionChangeType::Modified,
        EventKind::Remove(RemoveKind::File) => SessionChangeType::Deleted,
        _ => return Vec::new(),
    };

    event
        .paths
        .iter()
        .filter_map(|path| {
            // Only process .json files (not .lock or .tmp)
            if path.extension()?.to_str()? != "json" {
                return None;
            }
            let session_id = path.file_stem()?.to_str()?.to_string();
            Some(SessionChange {
                session_id,
                change_type,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{DataChange, ModifyKind};
    use std::path::PathBuf;

    #[test]
    fn test_app_event_enum() {
        // Just verify the enum variants can be created
        let _key = AppEvent::Key(KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::NONE,
        });
        let _changed = AppEvent::SessionsChanged(None);
        let _tick = AppEvent::Tick;
    }

    #[test]
    fn test_extract_session_changes_from_json_file() {
        let event = notify::Event {
            kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            paths: vec![PathBuf::from("/sessions/test-123.json")],
            attrs: Default::default(),
        };

        let changes = extract_session_changes(&event);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].session_id, "test-123");
        assert_eq!(changes[0].change_type, SessionChangeType::Modified);
    }

    #[test]
    fn test_extract_session_changes_ignores_lock_files() {
        let event = notify::Event {
            kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            paths: vec![PathBuf::from("/sessions/test-123.json.lock")],
            attrs: Default::default(),
        };

        let changes = extract_session_changes(&event);
        assert!(changes.is_empty());
    }

    #[test]
    fn test_extract_session_changes_ignores_tmp_files() {
        let event = notify::Event {
            kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            paths: vec![PathBuf::from("/sessions/test-123.json.tmp")],
            attrs: Default::default(),
        };

        let changes = extract_session_changes(&event);
        assert!(changes.is_empty());
    }

    #[test]
    fn test_extract_session_changes_create_event() {
        let event = notify::Event {
            kind: EventKind::Create(CreateKind::File),
            paths: vec![PathBuf::from("/sessions/new-session.json")],
            attrs: Default::default(),
        };

        let changes = extract_session_changes(&event);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].session_id, "new-session");
        assert_eq!(changes[0].change_type, SessionChangeType::Created);
    }

    #[test]
    fn test_extract_session_changes_delete_event() {
        let event = notify::Event {
            kind: EventKind::Remove(RemoveKind::File),
            paths: vec![PathBuf::from("/sessions/deleted-session.json")],
            attrs: Default::default(),
        };

        let changes = extract_session_changes(&event);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].session_id, "deleted-session");
        assert_eq!(changes[0].change_type, SessionChangeType::Deleted);
    }

    #[test]
    fn test_extract_session_changes_multiple_files() {
        let event = notify::Event {
            kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            paths: vec![
                PathBuf::from("/sessions/session-1.json"),
                PathBuf::from("/sessions/session-2.json"),
                PathBuf::from("/sessions/session-3.json.lock"), // should be ignored
            ],
            attrs: Default::default(),
        };

        let changes = extract_session_changes(&event);
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].session_id, "session-1");
        assert_eq!(changes[1].session_id, "session-2");
    }
}
