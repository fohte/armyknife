use anyhow::{Context, Result};
use notify_rust::Notification as RustNotification;

use super::types::Notification;

const FALLBACK_HINT: &str = "(Install Hammerspoon for click-to-focus)";

/// Sends a notification using notify-rust as a fallback.
/// Appends a hint about Hammerspoon if click actions are requested.
pub fn send(notification: &Notification) -> Result<()> {
    let message = if notification.action().is_some() {
        format!("{}\n{}", notification.message(), FALLBACK_HINT)
    } else {
        notification.message().to_string()
    };

    RustNotification::new()
        .summary(notification.title())
        .body(&message)
        .show()
        .context("failed to send notification via notify-rust")?;

    Ok(())
}
