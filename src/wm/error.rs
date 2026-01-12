use thiserror::Error;

use crate::git::GitError;

#[derive(Error, Debug)]
pub enum WmError {
    #[error("Not in a git repository")]
    NotInGitRepo,

    #[error("Worktree not found: {0}")]
    WorktreeNotFound(String),

    #[error("Operation cancelled")]
    Cancelled,

    #[error("Command failed: {0}")]
    CommandFailed(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON parse error: {0}")]
    JsonParse(#[from] serde_json::Error),

    #[error("Git error: {0}")]
    Git(#[from] GitError),
}

pub type Result<T> = std::result::Result<T, WmError>;
