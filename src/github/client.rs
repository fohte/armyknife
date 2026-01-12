//! GitHub API client implementation using octocrab.

use std::process::Command;
use std::sync::OnceLock;

use super::error::{GitHubError, Result};

/// Production implementation using octocrab.
pub struct OctocrabClient {
    pub(crate) client: octocrab::Octocrab,
}

/// Global singleton instance of OctocrabClient.
/// Initialized lazily on first access to avoid unnecessary `gh auth token` calls.
static OCTOCRAB_CLIENT: OnceLock<OctocrabClient> = OnceLock::new();

/// Stores initialization error if first attempt fails.
/// This ensures we don't retry initialization repeatedly on failure.
static OCTOCRAB_INIT_ERROR: OnceLock<String> = OnceLock::new();

impl OctocrabClient {
    /// Create a new OctocrabClient instance.
    /// Prefer using `OctocrabClient::get()` to reuse the singleton instance.
    fn new() -> Result<Self> {
        let token = get_gh_token()?;
        let client = octocrab::Octocrab::builder()
            .personal_token(token)
            .build()
            .map_err(|e| {
                GitHubError::TokenError(format!("Failed to build octocrab client: {e}"))
            })?;
        Ok(Self { client })
    }

    /// Get the singleton instance of OctocrabClient.
    /// Initializes the client on first call (runs `gh auth token` once).
    pub fn get() -> Result<&'static Self> {
        // Return cached client if available
        if let Some(client) = OCTOCRAB_CLIENT.get() {
            return Ok(client);
        }

        // Return cached error if initialization previously failed
        if let Some(err) = OCTOCRAB_INIT_ERROR.get() {
            return Err(GitHubError::TokenError(err.clone()));
        }

        // Try to initialize
        match Self::new() {
            Ok(client) => {
                // Use get_or_init to handle race conditions safely
                Ok(OCTOCRAB_CLIENT.get_or_init(|| client))
            }
            Err(e) => {
                // Cache the error message
                let _ = OCTOCRAB_INIT_ERROR.set(e.to_string());
                Err(e)
            }
        }
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
