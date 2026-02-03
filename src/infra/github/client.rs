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

    /// Get an issue by number using GraphQL.
    pub async fn get_issue(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
    ) -> Result<crate::commands::gh::issue_agent::models::Issue> {
        #[derive(Debug, Deserialize)]
        struct GetIssueData {
            repository: GetIssueRepositoryData,
        }

        #[derive(Debug, Deserialize)]
        struct GetIssueRepositoryData {
            issue: GraphQLIssue,
        }

        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct GraphQLIssue {
            number: i64,
            title: String,
            body: Option<String>,
            state: String,
            labels: LabelsConnection,
            assignees: AssigneesConnection,
            milestone: Option<MilestoneData>,
            author: Option<GraphQLAuthor>,
            created_at: chrono::DateTime<chrono::Utc>,
            updated_at: chrono::DateTime<chrono::Utc>,
        }

        #[derive(Debug, Deserialize)]
        struct LabelsConnection {
            nodes: Vec<LabelNode>,
        }

        #[derive(Debug, Deserialize)]
        struct LabelNode {
            name: String,
        }

        #[derive(Debug, Deserialize)]
        struct AssigneesConnection {
            nodes: Vec<AssigneeNode>,
        }

        #[derive(Debug, Deserialize)]
        struct AssigneeNode {
            login: String,
        }

        #[derive(Debug, Deserialize)]
        struct MilestoneData {
            title: String,
        }

        #[derive(Debug, Deserialize)]
        struct GraphQLAuthor {
            login: String,
        }

        const GET_ISSUE_QUERY: &str = indoc! {"
            query($owner: String!, $repo: String!, $number: Int!) {
                repository(owner: $owner, name: $repo) {
                    issue(number: $number) {
                        number
                        title
                        body
                        state
                        labels(first: 100) { nodes { name } }
                        assignees(first: 100) { nodes { login } }
                        milestone { title }
                        author { login }
                        createdAt
                        updatedAt
                    }
                }
            }
        "};

        let variables = serde_json::json!({
            "owner": owner,
            "repo": repo,
            "number": issue_number as i64,
        });

        let response: GetIssueData = self.graphql(GET_ISSUE_QUERY, variables).await?;
        let issue = response.repository.issue;

        Ok(crate::commands::gh::issue_agent::models::Issue {
            number: issue.number,
            title: issue.title,
            body: issue.body,
            state: issue.state,
            labels: issue
                .labels
                .nodes
                .into_iter()
                .map(|l| crate::commands::gh::issue_agent::models::Label { name: l.name })
                .collect(),
            assignees: issue
                .assignees
                .nodes
                .into_iter()
                .map(|a| crate::commands::gh::issue_agent::models::Author { login: a.login })
                .collect(),
            milestone: issue
                .milestone
                .map(|m| crate::commands::gh::issue_agent::models::Milestone { title: m.title }),
            author: issue
                .author
                .map(|a| crate::commands::gh::issue_agent::models::Author { login: a.login }),
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

    /// Create a new issue.
    pub async fn create_issue(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
        body: &str,
        labels: &[String],
        assignees: &[String],
    ) -> Result<crate::commands::gh::issue_agent::models::Issue> {
        let issues = self.client.issues(owner, repo);
        let mut builder = issues.create(title).body(body);

        if !labels.is_empty() {
            builder = builder.labels(labels.to_vec());
        }

        if !assignees.is_empty() {
            builder = builder.assignees(assignees.to_vec());
        }

        let issue = builder.send().await?;
        Ok(issue.into())
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
            updated_at: chrono::DateTime<chrono::Utc>,
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
                                updatedAt
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
                updated_at: c.updated_at,
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
            // For newly created comments, updated_at equals created_at if not provided
            updated_at: comment.updated_at.unwrap_or(comment.created_at),
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

    // ============ Timeline Operations ============

    /// Get timeline events for an issue using GraphQL.
    ///
    /// Fetches events like cross-references, label changes, assignments, etc.
    /// Unknown event types are automatically filtered out.
    pub async fn get_timeline_events(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
    ) -> Result<Vec<crate::commands::gh::issue_agent::models::TimelineItem>> {
        #[derive(Debug, Deserialize)]
        struct GetTimelineData {
            repository: TimelineRepositoryData,
        }

        #[derive(Debug, Deserialize)]
        struct TimelineRepositoryData {
            issue: TimelineIssueData,
        }

        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct TimelineIssueData {
            timeline_items: TimelineConnection,
        }

        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct TimelineConnection {
            nodes: Vec<crate::commands::gh::issue_agent::models::TimelineItem>,
            page_info: PageInfo,
        }

        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct PageInfo {
            has_next_page: bool,
            end_cursor: Option<String>,
        }

        const GET_TIMELINE_QUERY: &str = indoc! {"
            query($owner: String!, $repo: String!, $number: Int!, $cursor: String) {
                repository(owner: $owner, name: $repo) {
                    issue(number: $number) {
                        timelineItems(first: 100, after: $cursor, itemTypes: [
                            CROSS_REFERENCED_EVENT,
                            LABELED_EVENT,
                            UNLABELED_EVENT,
                            ASSIGNED_EVENT,
                            UNASSIGNED_EVENT,
                            CLOSED_EVENT,
                            REOPENED_EVENT
                        ]) {
                            nodes {
                                __typename
                                ... on CrossReferencedEvent {
                                    createdAt
                                    actor { login }
                                    willCloseTarget
                                    source {
                                        __typename
                                        ... on Issue {
                                            number
                                            title
                                            repository {
                                                name
                                                owner { login }
                                            }
                                        }
                                        ... on PullRequest {
                                            number
                                            title
                                            repository {
                                                name
                                                owner { login }
                                            }
                                        }
                                    }
                                }
                                ... on LabeledEvent {
                                    createdAt
                                    actor { login }
                                    label { name }
                                }
                                ... on UnlabeledEvent {
                                    createdAt
                                    actor { login }
                                    label { name }
                                }
                                ... on AssignedEvent {
                                    createdAt
                                    actor { login }
                                    assignee {
                                        ... on User { login }
                                    }
                                }
                                ... on UnassignedEvent {
                                    createdAt
                                    actor { login }
                                    assignee {
                                        ... on User { login }
                                    }
                                }
                                ... on ClosedEvent {
                                    createdAt
                                    actor { login }
                                }
                                ... on ReopenedEvent {
                                    createdAt
                                    actor { login }
                                }
                            }
                            pageInfo {
                                hasNextPage
                                endCursor
                            }
                        }
                    }
                }
            }
        "};

        let mut all_events = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let variables = serde_json::json!({
                "owner": owner,
                "repo": repo,
                "number": issue_number as i64,
                "cursor": cursor,
            });

            let response: GetTimelineData = self.graphql(GET_TIMELINE_QUERY, variables).await?;
            let connection = response.repository.issue.timeline_items;

            // Filter out Unknown events
            all_events.extend(connection.nodes.into_iter().filter(|e| !e.is_unknown()));

            if !connection.page_info.has_next_page {
                break;
            }
            cursor = connection.page_info.end_cursor;
        }

        Ok(all_events)
    }

    // ============ Issue Template Operations ============

    /// Get issue templates for a repository using GraphQL.
    ///
    /// Returns all issue templates configured in the repository's
    /// `.github/ISSUE_TEMPLATE/` directory.
    pub async fn get_issue_templates(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<crate::commands::gh::issue_agent::models::IssueTemplate>> {
        #[derive(Debug, Deserialize)]
        struct GetIssueTemplatesData {
            repository: Option<RepositoryData>,
        }

        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct RepositoryData {
            issue_templates: Option<Vec<GraphQLIssueTemplate>>,
        }

        #[derive(Debug, Deserialize)]
        struct GraphQLIssueTemplate {
            name: String,
            title: Option<String>,
            body: Option<String>,
            about: Option<String>,
            filename: Option<String>,
            labels: Option<LabelsConnection>,
            assignees: Option<AssigneesConnection>,
        }

        #[derive(Debug, Deserialize)]
        struct LabelsConnection {
            nodes: Option<Vec<LabelNode>>,
        }

        #[derive(Debug, Deserialize)]
        struct LabelNode {
            name: String,
        }

        #[derive(Debug, Deserialize)]
        struct AssigneesConnection {
            nodes: Option<Vec<AssigneeNode>>,
        }

        #[derive(Debug, Deserialize)]
        struct AssigneeNode {
            login: String,
        }

        const GET_ISSUE_TEMPLATES_QUERY: &str = indoc! {"
            query($owner: String!, $repo: String!) {
                repository(owner: $owner, name: $repo) {
                    issueTemplates {
                        name
                        title
                        body
                        about
                        filename
                        labels(first: 100) {
                            nodes { name }
                        }
                        assignees(first: 100) {
                            nodes { login }
                        }
                    }
                }
            }
        "};

        let variables = serde_json::json!({
            "owner": owner,
            "repo": repo,
        });

        let response: GetIssueTemplatesData =
            self.graphql(GET_ISSUE_TEMPLATES_QUERY, variables).await?;

        let templates = response
            .repository
            .and_then(|r| r.issue_templates)
            .unwrap_or_default()
            .into_iter()
            .map(
                |t| crate::commands::gh::issue_agent::models::IssueTemplate {
                    name: t.name,
                    title: t.title,
                    body: t.body,
                    about: t.about,
                    filename: t.filename,
                    labels: t
                        .labels
                        .and_then(|l| l.nodes)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|n| n.name)
                        .collect(),
                    assignees: t
                        .assignees
                        .and_then(|a| a.nodes)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|n| n.login)
                        .collect(),
                },
            )
            .collect();

        Ok(templates)
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

#[cfg(test)]
mod tests {
    use crate::commands::gh::issue_agent::models::IssueTemplate;
    use crate::infra::github::mock::GitHubMockServer;
    use rstest::rstest;

    mod get_issue_templates_tests {
        use super::*;

        #[rstest]
        #[tokio::test]
        async fn test_returns_empty_when_no_templates() {
            let mock = GitHubMockServer::start().await;
            mock.repo("owner", "repo")
                .graphql_issue_templates(&[])
                .await;

            let client = mock.client();
            let result = client.get_issue_templates("owner", "repo").await;

            assert!(result.is_ok());
            assert!(result.unwrap().is_empty());
        }

        #[rstest]
        #[tokio::test]
        async fn test_returns_single_template() {
            let mock = GitHubMockServer::start().await;
            let template = IssueTemplate {
                name: "Bug Report".to_string(),
                title: Some("[Bug]: ".to_string()),
                body: Some("## Description\n\nDescribe the bug".to_string()),
                about: Some("File a bug report".to_string()),
                filename: Some("bug_report.yml".to_string()),
                labels: vec!["bug".to_string()],
                assignees: vec![],
            };
            mock.repo("owner", "repo")
                .graphql_issue_templates(std::slice::from_ref(&template))
                .await;

            let client = mock.client();
            let result = client.get_issue_templates("owner", "repo").await;

            assert!(result.is_ok());
            let templates = result.unwrap();
            assert_eq!(templates.len(), 1);
            assert_eq!(templates[0].name, "Bug Report");
            assert_eq!(templates[0].title, Some("[Bug]: ".to_string()));
            assert_eq!(templates[0].labels, vec!["bug".to_string()]);
        }

        #[rstest]
        #[tokio::test]
        async fn test_returns_multiple_templates() {
            let mock = GitHubMockServer::start().await;
            let templates = vec![
                IssueTemplate {
                    name: "Bug Report".to_string(),
                    title: None,
                    body: None,
                    about: None,
                    filename: None,
                    labels: vec!["bug".to_string()],
                    assignees: vec![],
                },
                IssueTemplate {
                    name: "Feature Request".to_string(),
                    title: Some("[Feature]: ".to_string()),
                    body: Some("Describe the feature".to_string()),
                    about: Some("Suggest an idea".to_string()),
                    filename: None,
                    labels: vec!["enhancement".to_string()],
                    assignees: vec!["maintainer".to_string()],
                },
            ];
            mock.repo("owner", "repo")
                .graphql_issue_templates(&templates)
                .await;

            let client = mock.client();
            let result = client.get_issue_templates("owner", "repo").await;

            assert!(result.is_ok());
            let fetched = result.unwrap();
            assert_eq!(fetched.len(), 2);
            assert_eq!(fetched[0].name, "Bug Report");
            assert_eq!(fetched[1].name, "Feature Request");
            assert_eq!(fetched[1].assignees, vec!["maintainer".to_string()]);
        }
    }

    mod create_issue_tests {
        use super::*;

        #[rstest]
        #[tokio::test]
        async fn test_create_issue_success() {
            let mock = GitHubMockServer::start().await;
            mock.repo("owner", "repo")
                .issue(42)
                .title("New Issue")
                .body("Issue body")
                .labels(vec!["bug", "urgent"])
                .create()
                .await;

            let client = mock.client();
            let result = client
                .create_issue(
                    "owner",
                    "repo",
                    "New Issue",
                    "Issue body",
                    &["bug".to_string(), "urgent".to_string()],
                    &[],
                )
                .await;

            assert!(result.is_ok());
            let issue = result.unwrap();
            assert_eq!(issue.number, 42);
            assert_eq!(issue.title, "New Issue");
        }

        #[rstest]
        #[tokio::test]
        async fn test_create_issue_with_assignees() {
            let mock = GitHubMockServer::start().await;
            mock.repo("owner", "repo")
                .issue(123)
                .title("Task")
                .body("Description")
                .create()
                .await;

            let client = mock.client();
            let result = client
                .create_issue(
                    "owner",
                    "repo",
                    "Task",
                    "Description",
                    &[],
                    &["user1".to_string()],
                )
                .await;

            assert!(result.is_ok());
        }

        #[rstest]
        #[tokio::test]
        async fn test_create_issue_minimal() {
            let mock = GitHubMockServer::start().await;
            mock.repo("owner", "repo")
                .issue(1)
                .title("Minimal")
                .body("")
                .labels(vec![])
                .create()
                .await;

            let client = mock.client();
            let result = client
                .create_issue("owner", "repo", "Minimal", "", &[], &[])
                .await;

            assert!(result.is_ok());
        }
    }
}
