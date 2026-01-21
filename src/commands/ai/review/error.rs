//! Error types for review command.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ReviewError {
    #[error("Failed to get repository info: {0}")]
    RepoInfoError(String),

    #[error("Git error: {0}")]
    GitError(#[from] crate::infra::git::GitError),

    #[error("GitHub API error: {0}")]
    GitHubError(#[from] crate::infra::github::GitHubError),

    #[error("Timeout waiting for review after {0} seconds")]
    Timeout(u64),

    #[error("No open PR found for current branch")]
    NoPrFound,

    #[error("Failed to parse review timestamp: {0}")]
    TimestampParseError(String),

    #[error("Reviewer is unable to review this PR: {0}")]
    ReviewerUnable(String),

    #[error("Review has not started yet")]
    ReviewNotStarted,
}

pub type Result<T> = anyhow::Result<T>;
