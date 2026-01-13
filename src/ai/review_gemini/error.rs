//! Error types for review-gemini command.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ReviewGeminiError {
    #[error("Failed to get repository info: {0}")]
    RepoInfoError(String),

    #[error("Git error: {0}")]
    GitError(#[from] crate::git::GitError),

    #[error("GitHub API error: {0}")]
    GitHubError(#[from] crate::github::GitHubError),

    #[error("Failed to post comment: {0}")]
    CommentError(String),

    #[error("Timeout waiting for Gemini review after {0} seconds")]
    Timeout(u64),

    #[error("No open PR found for current branch")]
    NoPrFound,

    #[error("Failed to parse review timestamp: {0}")]
    TimestampParseError(String),
}

pub type Result<T> = std::result::Result<T, ReviewGeminiError>;
