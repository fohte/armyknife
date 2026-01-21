//! GitHub API client implementation using octocrab.

use std::process::Command;
use std::sync::OnceLock;

use anyhow::Context;
use serde::Deserialize;

use super::error::{GitHubError, Result};

/// Internal wrapper for GitHub GraphQL API responses.
///
/// Used internally by `graphql` method to handle the `data` wrapper and errors.
#[derive(Debug, Deserialize)]
struct GraphQLResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GraphQLError>>,
}

/// GraphQL error returned by GitHub API.
#[derive(Debug, Deserialize)]
struct GraphQLError {
    message: String,
}

/// Production implementation using octocrab.
pub struct OctocrabClient {
    pub(crate) client: octocrab::Octocrab,
}

/// Global singleton instance of OctocrabClient, initialized lazily.
///
/// Stores the `Result` of initialization. Using a single `OnceLock` for the result
/// ensures initialization logic runs only once, even across multiple threads.
static OCTOCRAB_CLIENT: OnceLock<std::result::Result<OctocrabClient, String>> = OnceLock::new();

impl OctocrabClient {
    /// Create a new OctocrabClient instance.
    /// Prefer using `OctocrabClient::get()` to reuse the singleton instance.
    fn new() -> Result<Self> {
        let token = get_gh_token()?;
        let client = octocrab::Octocrab::builder()
            .personal_token(token)
            .build()
            .context("Failed to build octocrab client")?;
        Ok(Self { client })
    }

    /// Get the singleton instance of OctocrabClient.
    /// Initializes the client on first call (runs `gh auth token` once).
    pub fn get() -> Result<&'static Self> {
        // get_or_init ensures the closure is only run once across all threads
        OCTOCRAB_CLIENT
            .get_or_init(|| Self::new().map_err(|e| e.to_string()))
            .as_ref()
            .map_err(|e| GitHubError::TokenError(e.clone()).into())
    }

    /// Execute a GraphQL query and deserialize the response.
    ///
    /// Automatically handles the `data` wrapper and `errors` field from GitHub
    /// GraphQL responses. Returns the unwrapped data on success, or an error
    /// if the response contains GraphQL errors.
    pub async fn graphql<T: serde::de::DeserializeOwned>(
        &self,
        query: &str,
        variables: serde_json::Value,
    ) -> Result<T> {
        let body = serde_json::json!({
            "query": query,
            "variables": variables,
        });
        let response: GraphQLResponse<T> = self.client.graphql(&body).await?;

        if let Some(errors) = response.errors {
            let messages: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
            return Err(GitHubError::GraphQLError(messages.join(", ")).into());
        }

        response
            .data
            .ok_or_else(|| GitHubError::GraphQLError("No data in response".to_string()).into())
    }
}

/// Get GitHub token from `gh auth token` command.
/// This reuses the authentication from GitHub CLI.
fn get_gh_token() -> Result<String> {
    let output = Command::new("gh")
        .args(["auth", "token"])
        .output()
        .context("Failed to run gh auth token")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GitHubError::TokenError(format!("gh auth token failed: {stderr}")).into());
    }

    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if token.is_empty() {
        return Err(
            GitHubError::TokenError("gh auth token returned empty token".to_string()).into(),
        );
    }

    Ok(token)
}
