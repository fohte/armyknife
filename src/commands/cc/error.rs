use thiserror::Error;

#[derive(Error, Debug)]
pub enum CcError {
    #[error("Unknown hook event: {0}")]
    UnknownHookEvent(String),

    #[error("No input from stdin")]
    NoStdinInput,

    #[error("Failed to get cache directory")]
    CacheDirNotFound,
}
