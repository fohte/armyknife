//! GitHub API client module using octocrab.
//!
//! Provides OctocrabClient for GitHub operations,
//! with authentication via `gh auth token`.

mod client;
pub(crate) mod error;
#[cfg(test)]
pub(crate) mod mock;
mod pr;
mod repo;

pub use client::OctocrabClient;
pub use error::GitHubError;
#[cfg(test)]
pub use mock::{GitHubMockServer, RemoteComment, RemoteTimelineEvent};
pub use pr::{CreatePrParams, PrClient, PrState, UpdatePrParams};
pub use repo::RepoClient;
