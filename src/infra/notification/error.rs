use thiserror::Error;

/// Errors that can occur during notification operations.
#[derive(Debug, Error)]
pub enum NotificationError {
    #[error("terminal-notifier command failed: {0}")]
    TerminalNotifierFailed(String),

    #[error("notify-rust failed: {0}")]
    NotifyRustFailed(String),
}
