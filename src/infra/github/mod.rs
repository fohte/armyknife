//! GitHub API client module using reqwest.
//!
//! Provides GitHubClient for GitHub operations,
//! with authentication via `gh auth token`.

mod client;
pub(crate) mod error;
#[cfg(test)]
pub(crate) mod mock;
mod pr;
mod repo;

pub use client::GitHubClient;
pub use error::GitHubError;
#[cfg(test)]
pub use mock::{GitHubMockServer, RemoteComment, RemoteTimelineEvent};
pub use pr::{BranchPrQuery, CreatePrParams, PrClient, PrInfo, PrState, UpdatePrParams};
pub use repo::RepoClient;
