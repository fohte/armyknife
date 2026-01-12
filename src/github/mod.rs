//! GitHub API client module using octocrab.
//!
//! Provides a trait-based abstraction for GitHub operations,
//! with authentication via `gh auth token`.

mod client;
mod error;
#[cfg(test)]
pub mod mock;
mod pr;
mod repo;

pub use client::OctocrabClient;
pub use error::GitHubError;
pub use pr::{CreatePrParams, PrClient, PrState};
pub use repo::RepoClient;

#[cfg(test)]
pub use mock::MockGitHubClient;
