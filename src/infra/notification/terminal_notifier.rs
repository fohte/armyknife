use std::process::Command;

use anyhow::{Context, Result, bail};

use super::types::Notification;

/// Sends a notification using terminal-notifier.
/// Supports click actions via the -execute flag.
pub fn send(notification: &Notification) -> Result<()> {
    let mut cmd = Command::new("terminal-notifier");

    cmd.arg("-title").arg(notification.title());
    cmd.arg("-message").arg(notification.message());

    // Add click action if present
    if let Some(action) = notification.action() {
        cmd.arg("-execute")
            .arg(format!("zsh -c \"{}\"", action.command()));
    }

    let output = cmd
        .output()
        .context("failed to execute terminal-notifier")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("terminal-notifier failed: {}", stderr);
    }

    Ok(())
}
