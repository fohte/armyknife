mod hammerspoon;
pub mod icon;
mod types;

pub use types::{Notification, NotificationAction};

use std::sync::OnceLock;

use anyhow::Result;

const HAMMERSPOON_MISSING_MESSAGE: &str = "Hammerspoon is not installed; notifications will be skipped. Install with: brew install --cask hammerspoon";

static HAMMERSPOON_WARNED: OnceLock<()> = OnceLock::new();

/// Sends a notification via Hammerspoon. No-op (with a one-shot warning) if Hammerspoon is missing.
pub fn send(notification: &Notification) -> Result<()> {
    if is_hammerspoon_available() {
        hammerspoon::send(notification)
    } else {
        warn_hammerspoon_missing();
        Ok(())
    }
}

/// Removes notifications belonging to the given group from the notification center.
/// Only works with Hammerspoon; silently does nothing if unavailable.
pub fn remove_group(group: &str) -> Result<()> {
    if is_hammerspoon_available() {
        hammerspoon::remove_group(group)
    } else {
        Ok(())
    }
}

/// Checks if the Hammerspoon CLI (`hs`) is available on the system.
fn is_hammerspoon_available() -> bool {
    hammerspoon::find_hs_path().is_some()
}

fn warn_hammerspoon_missing() {
    if HAMMERSPOON_WARNED.set(()).is_ok() {
        tracing::warn!("{}", HAMMERSPOON_MISSING_MESSAGE);
        eprintln!("[armyknife] warning: {HAMMERSPOON_MISSING_MESSAGE}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notification_creation() {
        let notification = Notification::new("Test Title", "Test Message");
        assert_eq!(notification.title(), "Test Title");
        assert_eq!(notification.message(), "Test Message");
        assert!(notification.action().is_none());
        assert!(notification.group().is_none());
        assert!(notification.app_icon().is_none());
    }

    #[test]
    fn test_notification_with_action() {
        let action = NotificationAction::new("tmux switch-client -t '%123'; open -a WezTerm");
        let notification =
            Notification::new("Test Title", "Test Message").with_action(action.clone());
        assert!(notification.action().is_some());
        assert_eq!(
            notification.action().as_ref().map(|a| a.command()),
            Some(action.command())
        );
    }

    #[test]
    fn test_notification_with_group() {
        let notification = Notification::new("Title", "Message").with_group("session-123");
        assert_eq!(notification.group(), Some("session-123"));
    }

    #[test]
    fn test_notification_with_app_icon() {
        let notification = Notification::new("Title", "Message").with_app_icon("/tmp/icon.png");
        assert_eq!(notification.app_icon(), Some("/tmp/icon.png"));
    }
}
