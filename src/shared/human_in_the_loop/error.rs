use std::io;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum HumanInTheLoopError {
    #[error("File not found: {0}")]
    FileNotFound(PathBuf),

    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("YAML parse error: {0}")]
    YamlParse(#[from] serde_yaml::Error),

    #[error("Command failed: {0}")]
    CommandFailed(String),

    #[error(
        "Terminal emulator failed to launch within {timeout_secs}s (likely sleeping or unavailable)"
    )]
    TerminalLaunchFailed { timeout_secs: u64 },

    #[error("Not approved. Run 'review' and set 'submit: true'")]
    NotApproved,

    #[error("File has been modified after approval. Run 'review' again")]
    ModifiedAfterApproval,
}

pub type Result<T> = std::result::Result<T, HumanInTheLoopError>;
