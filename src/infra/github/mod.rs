//! GitHub API client module using reqwest.
//!
//! Provides GitHubClient for GitHub operations,
//! with authentication via `gh auth token`.

mod client;
pub(crate) mod error;
#[cfg(test)]
pub(crate) mod mock;
mod pr;
mod pr_number;
mod pr_stack;
mod repo;

pub use client::GitHubClient;
pub use error::GitHubError;
#[cfg(test)]
pub use mock::{GitHubMockServer, RemoteComment, RemoteTimelineEvent};
pub use pr::{
    BranchPrQuery, CreatePrParams, CreatedPullRequest, PrBodyClient, PrCiState, PrClient, PrInfo,
    PrProgressInfo, PrState, UpdatePrBodyParams, UpdatePrParams,
};
pub use pr_number::PrNumberQuery;
pub use pr_stack::{PrStackClient, PullRequestStack};
pub use repo::RepoClient;
