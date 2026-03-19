//! Repository operations.

use serde::Deserialize;

use super::client::GitHubClient;
use super::error::Result;

/// Trait for repository operations.
pub trait RepoClient: Send + Sync {
    /// Check if a repository exists on GitHub.
    async fn repo_exists(&self, owner: &str, repo: &str) -> Result<bool>;

    /// Check if a repository is private.
    async fn is_repo_private(&self, owner: &str, repo: &str) -> Result<bool>;

    /// Get the default branch name from GitHub API.
    async fn get_default_branch(&self, owner: &str, repo: &str) -> Result<String>;
}

/// Minimal response type for repository info.
#[derive(Debug, Deserialize)]
struct RepoResponse {
    private: Option<bool>,
    default_branch: Option<String>,
}

impl RepoClient for GitHubClient {
    async fn repo_exists(&self, owner: &str, repo: &str) -> Result<bool> {
        let route = format!("/repos/{owner}/{repo}");
        let response = self.http.get(self.url(&route)).send().await?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(false);
        }
        if !status.is_success() {
            let body: serde_json::Value = response
                .json()
                .await
                .unwrap_or_else(|_| serde_json::json!({"message": "Unknown error"}));
            return Err(super::error::api_error_from_response(status.as_u16(), &body).into());
        }
        Ok(true)
    }

    async fn is_repo_private(&self, owner: &str, repo: &str) -> Result<bool> {
        let route = format!("/repos/{owner}/{repo}");
        let repository: RepoResponse = self.rest_get(&route).await?;
        // Default to private for safety (e.g., to avoid incorrectly flagging
        // Japanese text in a private repo as an error)
        Ok(repository.private.unwrap_or(true))
    }

    async fn get_default_branch(&self, owner: &str, repo: &str) -> Result<String> {
        let route = format!("/repos/{owner}/{repo}");
        let repository: RepoResponse = self.rest_get(&route).await?;
        Ok(repository
            .default_branch
            .unwrap_or_else(|| "main".to_string()))
    }
}
