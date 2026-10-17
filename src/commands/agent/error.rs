use thiserror::Error;

#[derive(Error, Debug)]
pub enum CcError {
    #[error("Not in a git repository")]
    NotInGitRepo,

    #[error("Cancelled: no prompt provided")]
    Cancelled,

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

    #[error("No tracked parent session is recorded for the current session")]
    NoTrackedParent,

    #[error("Parent session '{0}' is not in the tracked session list")]
    ParentSessionNotTracked(String),

    #[error("Cannot notify your own session '{0}'")]
    SelfNotify(String),

    #[error("Session '{0}' has no tmux information (was not started in tmux)")]
    NoTmuxInfo(String),

    #[error(
        "Neither ARMYKNIFE_SESSION_ID nor CODEX_SESSION_ID is set (not running inside an armyknife-tracked session)"
    )]
    SelfSessionUnknown,

    #[error("Session '{0}' has ended; refusing to notify (the user ended it intentionally)")]
    SessionEnded(String),

    #[error(
        "Session '{0}' has no messaging socket (its Claude Code process predates peer messaging support); cannot notify"
    )]
    NoMessagingSocket(String),
}
