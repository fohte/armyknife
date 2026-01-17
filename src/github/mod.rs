//! GitHub API client module using octocrab.
//!
//! Provides a trait-based abstraction for GitHub operations,
//! with authentication via `gh auth token`.

mod client;
mod comment;
mod error;
mod issue;
#[cfg(test)]
pub mod mock;
mod pr;
mod repo;

pub use client::OctocrabClient;
#[allow(unused_imports)]
pub use comment::CommentClient;
pub use error::GitHubError;
#[allow(unused_imports)]
pub use issue::IssueClient;
pub use pr::{CreatePrParams, PrClient, PrState};
pub use repo::RepoClient;

#[cfg(test)]
#[allow(unused_imports)]
pub use mock::{
    AddLabelsParams, CreateCommentParams, MockGitHubClient, RemoveLabelParams, UpdateCommentParams,
    UpdateIssueBodyParams, UpdateIssueTitleParams,
};
