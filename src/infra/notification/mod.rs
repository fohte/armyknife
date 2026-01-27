mod fallback;
mod terminal_notifier;
mod types;

pub use types::{Notification, NotificationAction};

use anyhow::Result;

use crate::shared::command::is_command_available;

/// Sends a notification using the best available method.
/// Prefers terminal-notifier for click actions, falls back to notify-rust.
pub fn send(notification: &Notification) -> Result<()> {
    if is_terminal_notifier_available() {
        terminal_notifier::send(notification)
    } else {
        fallback::send(notification)
    }
}

/// Checks if terminal-notifier is available on the system.
pub fn is_terminal_notifier_available() -> bool {
    is_command_available("terminal-notifier")
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
}
