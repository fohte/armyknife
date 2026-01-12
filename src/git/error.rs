//! Git error types.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum GitError {
    #[error("Not in a git repository")]
    NotInRepo,

    #[error("No remote 'origin' found")]
    NoOriginRemote,

    #[error("Could not parse GitHub URL: {0}")]
    InvalidGitHubUrl(String),

    #[error("Git error: {0}")]
    Git2(#[from] git2::Error),

    #[error("Command failed: {0}")]
    CommandFailed(String),
}

pub type Result<T> = std::result::Result<T, GitError>;
