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

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Git error: {0}")]
    Git2(#[from] git2::Error),

    #[error("Not found: {0}")]
    NotFound(String),
}

pub type Result<T> = anyhow::Result<T>;
