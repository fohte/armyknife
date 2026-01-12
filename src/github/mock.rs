//! Mock implementations for testing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::error::{GitHubError, Result};
use super::pr::{CreatePrParams, PrClient, PrInfo};
use super::repo::RepoClient;

/// Mock implementation for testing.
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

    #[allow(dead_code)]
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
impl RepoClient for MockGitHubClient {
    async fn is_repo_private(&self, owner: &str, repo: &str) -> Result<bool> {
        let key = format!("{owner}/{repo}");
        Ok(self.private_repos.get(&key).copied().unwrap_or(true))
    }
}

#[async_trait::async_trait]
impl PrClient for MockGitHubClient {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::pr::PrState;

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

    #[tokio::test]
    async fn mock_client_returns_branch_pr_info() {
        let client = MockGitHubClient::new().with_branch_pr(
            "owner",
            "repo",
            "feature",
            PrInfo {
                state: PrState::Open,
                url: "https://github.com/owner/repo/pull/1".to_string(),
            },
        );

        let pr_info = client
            .get_pr_for_branch("owner", "repo", "feature")
            .await
            .unwrap();
        assert!(pr_info.is_some());
        let pr_info = pr_info.unwrap();
        assert_eq!(pr_info.state, PrState::Open);

        // Unknown branch returns None
        let no_pr = client
            .get_pr_for_branch("owner", "repo", "unknown")
            .await
            .unwrap();
        assert!(no_pr.is_none());
    }
}
