//! GitHub API error types.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum GitHubError {
    #[error("Failed to get GitHub token: {0}")]
    TokenError(String),

    #[error("GitHub API error: {0}")]
    ApiError(#[from] octocrab::Error),

    #[error("PR created but no URL in response")]
    MissingPrUrl,
}

pub type Result<T> = std::result::Result<T, GitHubError>;
