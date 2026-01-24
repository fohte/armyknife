//! GitHub API client implementation using octocrab.

#[cfg(not(test))]
use std::process::Command;
use std::sync::OnceLock;

use anyhow::Context;
use indoc::indoc;
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

    /// Create a new OctocrabClient with a custom base URL (for testing with wiremock).
    #[cfg(test)]
    pub fn with_base_url(base_url: &str, token: &str) -> Result<Self> {
        let client = octocrab::Octocrab::builder()
            .personal_token(token.to_string())
            .base_uri(base_url)
            .context("Invalid base URL")?
            .build()
            .context("Failed to build octocrab client")?;
        Ok(Self { client })
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

    // ============ Issue Operations ============

    /// Get an issue by number.
    pub async fn get_issue(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
    ) -> Result<crate::commands::gh::issue_agent::models::Issue> {
        let issue = self.client.issues(owner, repo).get(issue_number).await?;

        // Convert octocrab::models::issues::Issue to our Issue model
        let state = match issue.state {
            octocrab::models::IssueState::Open => "OPEN".to_string(),
            octocrab::models::IssueState::Closed => "CLOSED".to_string(),
            // IssueState is #[non_exhaustive], so handle future variants
            _ => format!("{:?}", issue.state).to_uppercase(),
        };
        Ok(crate::commands::gh::issue_agent::models::Issue {
            number: issue.number as i64,
            title: issue.title,
            body: issue.body,
            state,
            labels: issue
                .labels
                .into_iter()
                .map(|l| crate::commands::gh::issue_agent::models::Label { name: l.name })
                .collect(),
            assignees: issue
                .assignees
                .into_iter()
                .map(|a| crate::commands::gh::issue_agent::models::Author { login: a.login })
                .collect(),
            milestone: issue
                .milestone
                .map(|m| crate::commands::gh::issue_agent::models::Milestone { title: m.title }),
            author: Some(crate::commands::gh::issue_agent::models::Author {
                login: issue.user.login,
            }),
            created_at: issue.created_at,
            updated_at: issue.updated_at,
        })
    }

    /// Update an issue's body.
    pub async fn update_issue_body(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        body: &str,
    ) -> Result<()> {
        self.client
            .issues(owner, repo)
            .update(issue_number)
            .body(body)
            .send()
            .await?;
        Ok(())
    }

    /// Update an issue's title.
    pub async fn update_issue_title(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        title: &str,
    ) -> Result<()> {
        self.client
            .issues(owner, repo)
            .update(issue_number)
            .title(title)
            .send()
            .await?;
        Ok(())
    }

    /// Add labels to an issue.
    pub async fn add_labels(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        labels: &[String],
    ) -> Result<()> {
        self.client
            .issues(owner, repo)
            .add_labels(issue_number, labels)
            .await?;
        Ok(())
    }

    /// Remove a label from an issue.
    pub async fn remove_label(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        label: &str,
    ) -> Result<()> {
        self.client
            .issues(owner, repo)
            .remove_label(issue_number, label)
            .await?;
        Ok(())
    }

    // ============ Comment Operations ============

    /// GraphQL data for fetching comments.
    async fn get_comments_inner(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
    ) -> Result<Vec<crate::commands::gh::issue_agent::models::Comment>> {
        #[derive(Debug, Deserialize)]
        struct GetCommentsData {
            repository: RepositoryData,
        }

        #[derive(Debug, Deserialize)]
        struct RepositoryData {
            issue: IssueData,
        }

        #[derive(Debug, Deserialize)]
        struct IssueData {
            comments: CommentsConnection,
        }

        #[derive(Debug, Deserialize)]
        struct CommentsConnection {
            nodes: Vec<GraphQLComment>,
        }

        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct GraphQLComment {
            id: String,
            database_id: i64,
            author: Option<GraphQLAuthor>,
            created_at: chrono::DateTime<chrono::Utc>,
            body: String,
        }

        #[derive(Debug, Deserialize)]
        struct GraphQLAuthor {
            login: String,
        }

        const GET_COMMENTS_QUERY: &str = indoc! {"
            query($owner: String!, $repo: String!, $number: Int!) {
                repository(owner: $owner, name: $repo) {
                    issue(number: $number) {
                        comments(first: 100) {
                            nodes {
                                id
                                databaseId
                                author { login }
                                createdAt
                                body
                            }
                        }
                    }
                }
            }
        "};

        let variables = serde_json::json!({
            "owner": owner,
            "repo": repo,
            "number": issue_number as i64,
        });

        let response: GetCommentsData = self.graphql(GET_COMMENTS_QUERY, variables).await?;

        Ok(response
            .repository
            .issue
            .comments
            .nodes
            .into_iter()
            .map(|c| crate::commands::gh::issue_agent::models::Comment {
                id: c.id,
                database_id: c.database_id,
                author: c
                    .author
                    .map(|a| crate::commands::gh::issue_agent::models::Author { login: a.login }),
                created_at: c.created_at,
                body: c.body,
            })
            .collect())
    }

    /// Get comments for an issue using GraphQL.
    /// Returns both node ID (for GraphQL) and database ID (for REST API).
    pub async fn get_comments(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
    ) -> Result<Vec<crate::commands::gh::issue_agent::models::Comment>> {
        self.get_comments_inner(owner, repo, issue_number).await
    }

    /// Update a comment using REST API.
    pub async fn update_comment(
        &self,
        owner: &str,
        repo: &str,
        comment_id: u64,
        body: &str,
    ) -> Result<()> {
        // Use REST API: PATCH /repos/{owner}/{repo}/issues/comments/{comment_id}
        let route = format!("/repos/{owner}/{repo}/issues/comments/{comment_id}");
        let _response: serde_json::Value = self
            .client
            .patch(route, Some(&serde_json::json!({ "body": body })))
            .await?;
        Ok(())
    }

    /// Create a new comment on an issue.
    pub async fn create_comment(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        body: &str,
    ) -> Result<crate::commands::gh::issue_agent::models::Comment> {
        let comment = self
            .client
            .issues(owner, repo)
            .create_comment(issue_number, body)
            .await?;

        Ok(crate::commands::gh::issue_agent::models::Comment {
            id: comment.node_id,
            database_id: comment.id.0 as i64,
            author: Some(crate::commands::gh::issue_agent::models::Author {
                login: comment.user.login,
            }),
            created_at: comment.created_at,
            body: comment.body.unwrap_or_default(),
        })
    }

    /// Delete a comment from an issue.
    pub async fn delete_comment(&self, owner: &str, repo: &str, comment_id: u64) -> Result<()> {
        // Use REST API: DELETE /repos/{owner}/{repo}/issues/comments/{comment_id}
        // GitHub returns 204 No Content, so we use _delete to get raw response
        // and just check for success status without parsing body
        let route = format!("/repos/{owner}/{repo}/issues/comments/{comment_id}");
        let uri = http::Uri::builder()
            .path_and_query(route)
            .build()
            .context("Failed to build URI")?;
        let response = self.client._delete(uri, None::<&()>).await?;
        // Check for error status and drop the response body
        octocrab::map_github_error(response).await.map(drop)?;
        Ok(())
    }

    /// Get the current authenticated user.
    pub async fn get_current_user(&self) -> Result<String> {
        let user = self.client.current().user().await?;
        Ok(user.login)
    }
}

/// Get GitHub token from `gh auth token` command.
/// This reuses the authentication from GitHub CLI.
///
/// # Errors
/// Returns an error when called during tests to prevent accidental real API calls.
/// Use `OctocrabClient::with_base_url` in tests instead.
fn get_gh_token() -> Result<String> {
    #[cfg(test)]
    return Err(GitHubError::TokenError(
        "get_gh_token should not be called in tests. Use OctocrabClient::with_base_url instead."
            .to_string(),
    )
    .into());

    #[cfg(not(test))]
    {
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
}
