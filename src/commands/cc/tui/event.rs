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

/// Events that can occur in the TUI.
pub enum AppEvent {
    /// A key was pressed.
    Key(KeyEvent),
    /// Session data changed on disk.
    SessionsChanged,
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
            // Only notify on relevant events
            let should_notify = matches!(
                event.kind,
                EventKind::Create(CreateKind::File)
                    | EventKind::Modify(ModifyKind::Data(_))
                    | EventKind::Modify(ModifyKind::Name(_))
                    | EventKind::Remove(RemoveKind::File)
            );

            if should_notify {
                let _ = tx.send(AppEvent::SessionsChanged);
            }
        }
    })?;

    watcher.watch(Path::new(&sessions_dir), RecursiveMode::NonRecursive)?;

    Ok(Some(watcher))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_event_enum() {
        // Just verify the enum variants can be created
        let _key = AppEvent::Key(KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::NONE,
        });
        let _changed = AppEvent::SessionsChanged;
        let _tick = AppEvent::Tick;
    }
}
