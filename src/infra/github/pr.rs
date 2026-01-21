//! Pull request operations.

use super::client::OctocrabClient;
use super::error::{GitHubError, Result};

/// Parameters for creating a pull request.
#[derive(Debug, Clone)]
pub struct CreatePrParams {
    pub owner: String,
    pub repo: String,
    pub title: String,
    pub body: String,
    pub head: String,
    pub base: Option<String>,
    pub draft: bool,
}

/// PR state from GitHub API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrState {
    Open,
    Closed,
    Merged,
}

/// PR information from GitHub API.
#[derive(Debug, Clone)]
pub struct PrInfo {
    pub state: PrState,
    pub url: String,
}

/// Trait for pull request operations.
#[async_trait::async_trait]
pub trait PrClient: Send + Sync {
    /// Create a pull request and return its URL.
    async fn create_pull_request(&self, params: CreatePrParams) -> Result<String>;

    /// Get PR state for a branch. Returns None if no PR exists.
    async fn get_pr_for_branch(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<Option<PrInfo>>;

    /// Open a URL in the default browser.
    fn open_in_browser(&self, url: &str);
}

#[async_trait::async_trait]
impl PrClient for OctocrabClient {
    async fn create_pull_request(&self, params: CreatePrParams) -> Result<String> {
        let pulls = self.client.pulls(&params.owner, &params.repo);

        // If base is not specified, find the base branch from local git info or GitHub API
        let base = match &params.base {
            Some(b) => b.clone(),
            None => crate::infra::git::find_base_branch(&params.owner, &params.repo).await,
        };

        let pr = if params.draft {
            pulls
                .create(&params.title, &params.head, &base)
                .body(&params.body)
                .draft(Some(true))
                .send()
                .await?
        } else {
            pulls
                .create(&params.title, &params.head, &base)
                .body(&params.body)
                .send()
                .await?
        };

        pr.html_url
            .map(|u| u.to_string())
            .ok_or_else(|| GitHubError::MissingPrUrl.into())
    }

    async fn get_pr_for_branch(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<Option<PrInfo>> {
        // Search for PRs with this head branch
        let pulls = self
            .client
            .pulls(owner, repo)
            .list()
            .head(format!("{owner}:{branch}"))
            .state(octocrab::params::State::All)
            .send()
            .await?;

        // Get the first (most recent) PR for this branch
        let Some(pr) = pulls.items.into_iter().next() else {
            return Ok(None);
        };

        let state = if pr.merged_at.is_some() {
            PrState::Merged
        } else {
            match pr.state {
                Some(octocrab::models::IssueState::Open) => PrState::Open,
                Some(octocrab::models::IssueState::Closed) => PrState::Closed,
                _ => PrState::Closed,
            }
        };

        let url = pr.html_url.map(|u| u.to_string()).unwrap_or_default();

        Ok(Some(PrInfo { state, url }))
    }

    fn open_in_browser(&self, url: &str) {
        let _ = open::that(url);
    }
}
