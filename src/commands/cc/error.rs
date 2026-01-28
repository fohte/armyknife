use thiserror::Error;

#[derive(Error, Debug)]
pub enum CcError {
    #[error("Unknown hook event: {0}")]
    UnknownHookEvent(String),

    #[error("No input from stdin")]
    NoStdinInput,

    #[error("Failed to parse JSON from stdin: {0}")]
    JsonParseError(#[from] serde_json::Error),

    #[error("Failed to get cache directory")]
    CacheDirNotFound,

    #[error("Invalid session ID: {0}")]
    InvalidSessionId(String),

    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Session '{0}' has no tmux information (was not started in tmux)")]
    NoTmuxInfo(String),
}
