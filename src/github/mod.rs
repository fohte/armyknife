//! GitHub API client module using octocrab.
//!
//! Provides a trait-based abstraction for GitHub operations,
//! with authentication via `gh auth token`.

use std::process::Command;
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

/// Trait for GitHub API operations.
/// Enables dependency injection for testing without network calls.
#[async_trait::async_trait]
pub trait GitHubClient: Send + Sync {
    /// Check if a repository is private.
    async fn is_repo_private(&self, owner: &str, repo: &str) -> Result<bool>;

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

/// Production implementation using octocrab.
pub struct OctocrabClient {
    client: octocrab::Octocrab,
}

impl OctocrabClient {
    pub fn new() -> Result<Self> {
        let token = get_gh_token()?;
        let client = octocrab::Octocrab::builder()
            .personal_token(token)
            .build()
            .map_err(|e| {
                GitHubError::TokenError(format!("Failed to build octocrab client: {e}"))
            })?;
        Ok(Self { client })
    }
}

#[async_trait::async_trait]
impl GitHubClient for OctocrabClient {
    async fn is_repo_private(&self, owner: &str, repo: &str) -> Result<bool> {
        let repository = self.client.repos(owner, repo).get().await?;
        // Default to private for safety (e.g., to avoid incorrectly flagging
        // Japanese text in a private repo as an error)
        Ok(repository.private.unwrap_or(true))
    }

    async fn create_pull_request(&self, params: CreatePrParams) -> Result<String> {
        let pulls = self.client.pulls(&params.owner, &params.repo);
        let base = params.base.as_deref().unwrap_or("main");

        let pr = if params.draft {
            pulls
                .create(&params.title, &params.head, base)
                .body(&params.body)
                .draft(Some(true))
                .send()
                .await?
        } else {
            pulls
                .create(&params.title, &params.head, base)
                .body(&params.body)
                .send()
                .await?
        };

        pr.html_url
            .map(|u| u.to_string())
            .ok_or_else(|| GitHubError::MissingPrUrl)
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

/// Get GitHub token from `gh auth token` command.
/// This reuses the authentication from GitHub CLI.
fn get_gh_token() -> Result<String> {
    let output = Command::new("gh")
        .args(["auth", "token"])
        .output()
        .map_err(|e| GitHubError::TokenError(format!("Failed to run gh auth token: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GitHubError::TokenError(format!(
            "gh auth token failed: {stderr}"
        )));
    }

    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if token.is_empty() {
        return Err(GitHubError::TokenError(
            "gh auth token returned empty token".to_string(),
        ));
    }

    Ok(token)
}

/// Test utilities for mocking GitHub API.
#[cfg(test)]
pub mod test_utils {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    /// Mock implementation of GitHubClient for testing.
    #[derive(Clone)]
    pub struct MockGitHubClient {
        /// Map of "owner/repo" -> is_private
        pub private_repos: HashMap<String, bool>,
        /// Result URL for PR creation (None = error)
        pub pr_create_result: Option<String>,
        /// Map of "owner/repo/branch" -> PrInfo
        pub branch_prs: HashMap<String, PrInfo>,
        /// Track created PRs for assertions
        pub created_prs: Arc<Mutex<Vec<CreatePrParams>>>,
        /// Track browser opens for assertions
        pub opened_urls: Arc<Mutex<Vec<String>>>,
    }

    impl MockGitHubClient {
        pub fn new() -> Self {
            Self {
                private_repos: HashMap::new(),
                pr_create_result: Some("https://github.com/owner/repo/pull/1".to_string()),
                branch_prs: HashMap::new(),
                created_prs: Arc::new(Mutex::new(Vec::new())),
                opened_urls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        pub fn with_private(mut self, owner: &str, repo: &str, is_private: bool) -> Self {
            self.private_repos
                .insert(format!("{owner}/{repo}"), is_private);
            self
        }

        pub fn with_pr_result(mut self, result: Option<String>) -> Self {
            self.pr_create_result = result;
            self
        }

        pub fn with_branch_pr(
            mut self,
            owner: &str,
            repo: &str,
            branch: &str,
            pr_info: PrInfo,
        ) -> Self {
            self.branch_prs
                .insert(format!("{owner}/{repo}/{branch}"), pr_info);
            self
        }
    }

    #[async_trait::async_trait]
    impl GitHubClient for MockGitHubClient {
        async fn is_repo_private(&self, owner: &str, repo: &str) -> Result<bool> {
            let key = format!("{owner}/{repo}");
            Ok(self.private_repos.get(&key).copied().unwrap_or(true))
        }

        async fn create_pull_request(&self, params: CreatePrParams) -> Result<String> {
            self.created_prs.lock().unwrap().push(params);
            self.pr_create_result
                .clone()
                .ok_or_else(|| GitHubError::TokenError("Mock PR creation failed".to_string()))
        }

        async fn get_pr_for_branch(
            &self,
            owner: &str,
            repo: &str,
            branch: &str,
        ) -> Result<Option<PrInfo>> {
            let key = format!("{owner}/{repo}/{branch}");
            Ok(self.branch_prs.get(&key).cloned())
        }

        fn open_in_browser(&self, url: &str) {
            self.opened_urls.lock().unwrap().push(url.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_utils::*;
    use super::*;

    #[tokio::test]
    async fn mock_client_returns_configured_private_status() {
        let client = MockGitHubClient::new()
            .with_private("owner", "public-repo", false)
            .with_private("owner", "private-repo", true);

        assert!(
            !client
                .is_repo_private("owner", "public-repo")
                .await
                .unwrap()
        );
        assert!(
            client
                .is_repo_private("owner", "private-repo")
                .await
                .unwrap()
        );
        // Default to private for unknown repos
        assert!(client.is_repo_private("owner", "unknown").await.unwrap());
    }

    #[tokio::test]
    async fn mock_client_tracks_created_prs() {
        let client = MockGitHubClient::new();

        let params = CreatePrParams {
            owner: "owner".to_string(),
            repo: "repo".to_string(),
            title: "Test PR".to_string(),
            body: "Test body".to_string(),
            head: "feature".to_string(),
            base: Some("main".to_string()),
            draft: false,
        };

        let url = client.create_pull_request(params).await.unwrap();
        assert_eq!(url, "https://github.com/owner/repo/pull/1");

        let created = client.created_prs.lock().unwrap();
        assert_eq!(created.len(), 1);
        assert_eq!(created[0].title, "Test PR");
    }

    #[test]
    fn mock_client_tracks_browser_opens() {
        let client = MockGitHubClient::new();
        client.open_in_browser("https://example.com");

        let opened = client.opened_urls.lock().unwrap();
        assert_eq!(opened.len(), 1);
        assert_eq!(opened[0], "https://example.com");
    }
}
