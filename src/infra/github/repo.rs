//! Repository operations.

use http::StatusCode;

use super::client::OctocrabClient;
use super::error::Result;

/// Trait for repository operations.
#[async_trait::async_trait]
pub trait RepoClient: Send + Sync {
    /// Check if a repository exists on GitHub.
    async fn repo_exists(&self, owner: &str, repo: &str) -> Result<bool>;

    /// Check if a repository is private.
    async fn is_repo_private(&self, owner: &str, repo: &str) -> Result<bool>;

    /// Get the default branch name from GitHub API.
    async fn get_default_branch(&self, owner: &str, repo: &str) -> Result<String>;
}

#[async_trait::async_trait]
impl RepoClient for OctocrabClient {
    async fn repo_exists(&self, owner: &str, repo: &str) -> Result<bool> {
        match self.client.repos(owner, repo).get().await {
            Ok(_) => Ok(true),
            Err(octocrab::Error::GitHub { source, .. })
                if source.status_code == StatusCode::NOT_FOUND =>
            {
                Ok(false)
            }
            Err(e) => Err(e.into()),
        }
    }

    async fn is_repo_private(&self, owner: &str, repo: &str) -> Result<bool> {
        let repository = self.client.repos(owner, repo).get().await?;
        // Default to private for safety (e.g., to avoid incorrectly flagging
        // Japanese text in a private repo as an error)
        Ok(repository.private.unwrap_or(true))
    }

    async fn get_default_branch(&self, owner: &str, repo: &str) -> Result<String> {
        let repository = self.client.repos(owner, repo).get().await?;
        Ok(repository
            .default_branch
            .unwrap_or_else(|| "main".to_string()))
    }
}
