//! GitHub API client implementation using reqwest.

#[cfg(not(test))]
use std::process::Command;
use std::sync::OnceLock;

use anyhow::Context;
use indoc::indoc;
use serde::Deserialize;

use super::error::{GitHubError, Result, api_error_from_response};

const GITHUB_API_BASE: &str = "https://api.github.com";
const GITHUB_GRAPHQL_URL: &str = "https://api.github.com/graphql";

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

/// GitHub API client using reqwest.
pub struct GitHubClient {
    pub(crate) http: reqwest::Client,
    pub(crate) base_url: String,
    pub(crate) graphql_url: String,
}

/// Global singleton instance of GitHubClient, initialized lazily.
///
/// Stores the `Result` of initialization. Using a single `OnceLock` for the result
/// ensures initialization logic runs only once, even across multiple threads.
static GITHUB_CLIENT: OnceLock<std::result::Result<GitHubClient, String>> = OnceLock::new();

impl GitHubClient {
    /// Create a new GitHubClient instance.
    /// Prefer using `GitHubClient::get()` to reuse the singleton instance.
    fn new() -> Result<Self> {
        let token = get_gh_token()?;
        let http = reqwest::Client::builder()
            .default_headers({
                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert(
                    reqwest::header::AUTHORIZATION,
                    reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
                        .context("Invalid token")?,
                );
                headers.insert(
                    reqwest::header::ACCEPT,
                    reqwest::header::HeaderValue::from_static("application/vnd.github+json"),
                );
                headers.insert(
                    reqwest::header::USER_AGENT,
                    reqwest::header::HeaderValue::from_static("armyknife"),
                );
                headers
            })
            .build()
            .context("Failed to build HTTP client")?;
        Ok(Self {
            http,
            base_url: GITHUB_API_BASE.to_string(),
            graphql_url: GITHUB_GRAPHQL_URL.to_string(),
        })
    }

    /// Get the singleton instance of GitHubClient.
    /// Initializes the client on first call (runs `gh auth token` once).
    pub fn get() -> Result<&'static Self> {
        // get_or_init ensures the closure is only run once across all threads
        GITHUB_CLIENT
            .get_or_init(|| Self::new().map_err(|e| e.to_string()))
            .as_ref()
            .map_err(|e| GitHubError::TokenError(e.clone()).into())
    }

    /// Create a new GitHubClient with a custom base URL (for testing with wiremock).
    #[cfg(test)]
    pub fn with_base_url(base_url: &str, token: &str) -> Result<Self> {
        let http = reqwest::Client::builder()
            .default_headers({
                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert(
                    reqwest::header::AUTHORIZATION,
                    reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
                        .context("Invalid token")?,
                );
                headers.insert(
                    reqwest::header::ACCEPT,
                    reqwest::header::HeaderValue::from_static("application/vnd.github+json"),
                );
                headers.insert(
                    reqwest::header::USER_AGENT,
                    reqwest::header::HeaderValue::from_static("armyknife"),
                );
                headers
            })
            .build()
            .context("Failed to build HTTP client")?;
        let graphql_url = format!("{base_url}/graphql");
        Ok(Self {
            http,
            base_url: base_url.to_string(),
            graphql_url,
        })
    }

    /// Build a full URL for a REST API route.
    pub(crate) fn url(&self, route: &str) -> String {
        format!("{}{route}", self.base_url)
    }

    /// Send a GET request and deserialize the JSON response.
    pub(crate) async fn rest_get<T: serde::de::DeserializeOwned>(&self, route: &str) -> Result<T> {
        let response = self.http.get(self.url(route)).send().await?;
        check_response(response).await
    }

    /// Send a GET request with query parameters and deserialize the JSON response.
    pub(crate) async fn rest_get_with_query<T: serde::de::DeserializeOwned>(
        &self,
        route: &str,
        query: &[(&str, &str)],
    ) -> Result<T> {
        let response = self.http.get(self.url(route)).query(query).send().await?;
        check_response(response).await
    }

    /// Send a POST request with a JSON body and deserialize the response.
    pub(crate) async fn rest_post<T: serde::de::DeserializeOwned>(
        &self,
        route: &str,
        body: &serde_json::Value,
    ) -> Result<T> {
        let response = self.http.post(self.url(route)).json(body).send().await?;
        check_response(response).await
    }

    /// Send a PATCH request with a JSON body and deserialize the response.
    pub(crate) async fn rest_patch<T: serde::de::DeserializeOwned>(
        &self,
        route: &str,
        body: &serde_json::Value,
    ) -> Result<T> {
        let response = self.http.patch(self.url(route)).json(body).send().await?;
        check_response(response).await
    }

    /// Send a DELETE request. Expects 204 No Content or similar success status.
    pub(crate) async fn rest_delete(&self, route: &str) -> Result<()> {
        let response = self.http.delete(self.url(route)).send().await?;
        let status = response.status();
        if !status.is_success() {
            let body: serde_json::Value = response
                .json()
                .await
                .unwrap_or_else(|_| serde_json::json!({"message": "Unknown error"}));
            return Err(api_error_from_response(status.as_u16(), &body).into());
        }
        Ok(())
    }

    /// Send a DELETE request with a JSON body. Expects success status.
    pub(crate) async fn rest_delete_with_body(
        &self,
        route: &str,
        body: &serde_json::Value,
    ) -> Result<()> {
        let response = self.http.delete(self.url(route)).json(body).send().await?;
        let status = response.status();
        if !status.is_success() {
            let body: serde_json::Value = response
                .json()
                .await
                .unwrap_or_else(|_| serde_json::json!({"message": "Unknown error"}));
            return Err(api_error_from_response(status.as_u16(), &body).into());
        }
        Ok(())
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
        let response = self.http.post(&self.graphql_url).json(&body).send().await?;
        let response: GraphQLResponse<T> = check_response(response).await?;

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
    /// This fetches additional fields not available via REST API:
    /// - lastEditedAt: timestamp when the issue body was last edited
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
            last_edited_at: Option<chrono::DateTime<chrono::Utc>>,
            parent: Option<ParentIssueData>,
        }

        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ParentIssueData {
            number: i64,
            repository: ParentRepoData,
        }

        #[derive(Debug, Deserialize)]
        struct ParentRepoData {
            name: String,
            owner: ParentRepoOwner,
        }

        #[derive(Debug, Deserialize)]
        struct ParentRepoOwner {
            login: String,
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
                        lastEditedAt
                        parent {
                            number
                            repository {
                                name
                                owner { login }
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
            last_edited_at: issue.last_edited_at,
            parent_issue: issue.parent.map(|p| {
                crate::commands::gh::issue_agent::models::SubIssueRef {
                    // GraphQL doesn't expose the REST API numeric id for parent
                    id: 0,
                    number: p.number,
                    owner: p.repository.owner.login,
                    repo: p.repository.name,
                }
            }),
            // Populated separately by get_sub_issues
            sub_issues: vec![],
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
        let route = format!("/repos/{owner}/{repo}/issues/{issue_number}");
        let _: serde_json::Value = self
            .rest_patch(&route, &serde_json::json!({ "body": body }))
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
        let route = format!("/repos/{owner}/{repo}/issues/{issue_number}");
        let _: serde_json::Value = self
            .rest_patch(&route, &serde_json::json!({ "title": title }))
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
        let route = format!("/repos/{owner}/{repo}/issues/{issue_number}/labels");
        let _: serde_json::Value = self
            .rest_post(&route, &serde_json::json!({ "labels": labels }))
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
        let encoded_label =
            percent_encoding::utf8_percent_encode(label, percent_encoding::NON_ALPHANUMERIC);
        let route = format!("/repos/{owner}/{repo}/issues/{issue_number}/labels/{encoded_label}");
        // GitHub returns 200 with remaining labels, not 204
        let response = self.http.delete(self.url(&route)).send().await?;
        let status = response.status();
        if !status.is_success() {
            let body: serde_json::Value = response
                .json()
                .await
                .unwrap_or_else(|_| serde_json::json!({"message": "Unknown error"}));
            return Err(api_error_from_response(status.as_u16(), &body).into());
        }
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
        let route = format!("/repos/{owner}/{repo}/issues");
        let mut payload = serde_json::json!({
            "title": title,
            "body": body,
        });

        if !labels.is_empty() {
            payload["labels"] = serde_json::json!(labels);
        }

        if !assignees.is_empty() {
            payload["assignees"] = serde_json::json!(assignees);
        }

        let response: CreateIssueResponse = self.rest_post(&route, &payload).await?;

        Ok(crate::commands::gh::issue_agent::models::Issue {
            number: response.number,
            title: response.title,
            body: response.body,
            state: match response.state.as_str() {
                "open" => "OPEN".to_string(),
                "closed" => "CLOSED".to_string(),
                other => other.to_uppercase(),
            },
            labels: response
                .labels
                .into_iter()
                .map(|l| crate::commands::gh::issue_agent::models::Label { name: l.name })
                .collect(),
            assignees: response
                .assignees
                .into_iter()
                .map(|a| crate::commands::gh::issue_agent::models::Author { login: a.login })
                .collect(),
            milestone: response
                .milestone
                .map(|m| crate::commands::gh::issue_agent::models::Milestone { title: m.title }),
            author: Some(crate::commands::gh::issue_agent::models::Author {
                login: response.user.login,
            }),
            created_at: response.created_at,
            updated_at: response.updated_at,
            last_edited_at: None,
            parent_issue: None,
            sub_issues: vec![],
        })
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
        let route = format!("/repos/{owner}/{repo}/issues/comments/{comment_id}");
        let _: serde_json::Value = self
            .rest_patch(&route, &serde_json::json!({ "body": body }))
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
        let route = format!("/repos/{owner}/{repo}/issues/{issue_number}/comments");
        let comment: CreateCommentResponse = self
            .rest_post(&route, &serde_json::json!({ "body": body }))
            .await?;

        Ok(crate::commands::gh::issue_agent::models::Comment {
            id: comment.node_id,
            database_id: comment.id as i64,
            author: Some(crate::commands::gh::issue_agent::models::Author {
                login: comment.user.login,
            }),
            created_at: comment.created_at,
            updated_at: comment.updated_at.unwrap_or(comment.created_at),
            body: comment.body.unwrap_or_default(),
        })
    }

    /// Delete a comment from an issue.
    pub async fn delete_comment(&self, owner: &str, repo: &str, comment_id: u64) -> Result<()> {
        let route = format!("/repos/{owner}/{repo}/issues/comments/{comment_id}");
        self.rest_delete(&route).await
    }

    /// Get the current authenticated user.
    pub async fn get_current_user(&self) -> Result<String> {
        #[derive(Deserialize)]
        struct User {
            login: String,
        }
        let user: User = self.rest_get("/user").await?;
        Ok(user.login)
    }

    // ============ Sub-Issue Operations ============

    /// Get sub-issues for an issue using REST API.
    pub async fn get_sub_issues(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
    ) -> Result<Vec<crate::commands::gh::issue_agent::models::SubIssueRef>> {
        #[derive(Debug, Deserialize)]
        struct SubIssueResponse {
            id: u64,
            number: i64,
            repository_url: String,
        }

        let route = format!("/repos/{owner}/{repo}/issues/{issue_number}/sub_issues?per_page=100");
        let response: Vec<SubIssueResponse> = self.rest_get(&route).await?;

        Ok(response
            .into_iter()
            .map(|s| {
                let (sub_owner, sub_repo) = parse_repository_url(&s.repository_url);
                crate::commands::gh::issue_agent::models::SubIssueRef {
                    id: s.id,
                    number: s.number,
                    owner: sub_owner,
                    repo: sub_repo,
                }
            })
            .collect())
    }

    /// Add a sub-issue to a parent issue.
    /// `sub_issue_id` is the internal issue ID (not the issue number).
    pub async fn add_sub_issue(
        &self,
        owner: &str,
        repo: &str,
        parent_issue_number: u64,
        sub_issue_id: u64,
    ) -> Result<()> {
        let route = format!("/repos/{owner}/{repo}/issues/{parent_issue_number}/sub_issues");
        let _: serde_json::Value = self
            .rest_post(&route, &serde_json::json!({ "sub_issue_id": sub_issue_id }))
            .await?;
        Ok(())
    }

    /// Remove a sub-issue from a parent issue.
    /// `sub_issue_id` is the internal issue ID (not the issue number).
    pub async fn remove_sub_issue(
        &self,
        owner: &str,
        repo: &str,
        parent_issue_number: u64,
        sub_issue_id: u64,
    ) -> Result<()> {
        // Note: endpoint uses singular "sub_issue" for DELETE
        let route = format!("/repos/{owner}/{repo}/issues/{parent_issue_number}/sub_issue");
        self.rest_delete_with_body(&route, &serde_json::json!({ "sub_issue_id": sub_issue_id }))
            .await
    }

    /// Get the internal ID for an issue (needed for Sub-issues API).
    pub async fn get_issue_id(&self, owner: &str, repo: &str, issue_number: u64) -> Result<u64> {
        let route = format!("/repos/{owner}/{repo}/issues/{issue_number}");
        let response: serde_json::Value = self.rest_get(&route).await?;
        let id = response["id"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("Missing 'id' field in issue response"))?;
        Ok(id)
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

    // ============ PR Review Operations ============

    /// Reply to a PR review comment using REST API.
    pub async fn reply_to_pr_review_comment(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        in_reply_to: i64,
        body: &str,
    ) -> Result<()> {
        let route = format!("/repos/{owner}/{repo}/pulls/{pr_number}/comments");
        let _: serde_json::Value = self
            .rest_post(
                &route,
                &serde_json::json!({
                    "body": body,
                    "in_reply_to": in_reply_to,
                }),
            )
            .await?;
        Ok(())
    }

    /// Resolve a review thread using GraphQL mutation.
    pub async fn resolve_review_thread(&self, thread_node_id: &str) -> Result<()> {
        const RESOLVE_MUTATION: &str = indoc! {"
            mutation($threadId: ID!) {
                resolveReviewThread(input: { threadId: $threadId }) {
                    thread {
                        id
                        isResolved
                    }
                }
            }
        "};

        let variables = serde_json::json!({
            "threadId": thread_node_id,
        });

        let _: serde_json::Value = self.graphql(RESOLVE_MUTATION, variables).await?;
        Ok(())
    }
}

/// Response types for REST API deserialization.
#[derive(Debug, Deserialize)]
struct CreateIssueResponse {
    number: i64,
    title: String,
    body: Option<String>,
    state: String,
    labels: Vec<CreateIssueLabelResponse>,
    assignees: Vec<CreateIssueUserResponse>,
    milestone: Option<CreateIssueMilestoneResponse>,
    user: CreateIssueUserResponse,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
struct CreateIssueLabelResponse {
    name: String,
}

#[derive(Debug, Deserialize)]
struct CreateIssueUserResponse {
    login: String,
}

#[derive(Debug, Deserialize)]
struct CreateIssueMilestoneResponse {
    title: String,
}

#[derive(Debug, Deserialize)]
struct CreateCommentResponse {
    id: u64,
    node_id: String,
    body: Option<String>,
    user: CreateIssueUserResponse,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Check HTTP response status and deserialize JSON body, or return an error.
async fn check_response<T: serde::de::DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    let status = response.status();
    if !status.is_success() {
        let body: serde_json::Value = response
            .json()
            .await
            .unwrap_or_else(|_| serde_json::json!({"message": "Unknown error"}));
        return Err(api_error_from_response(status.as_u16(), &body).into());
    }
    let body = response.json().await?;
    Ok(body)
}

/// Parse owner and repo from a GitHub repository URL.
/// Input: "https://api.github.com/repos/{owner}/{repo}"
/// Returns: (owner, repo)
fn parse_repository_url(url: &str) -> (String, String) {
    let mut parts = url.rsplitn(3, '/');
    match (parts.next(), parts.next()) {
        (Some(repo), Some(owner)) if !repo.is_empty() && !owner.is_empty() => {
            (owner.to_string(), repo.to_string())
        }
        _ => ("unknown".to_string(), "unknown".to_string()),
    }
}

/// Get GitHub token from `gh auth token` command.
/// This reuses the authentication from GitHub CLI.
///
/// # Errors
/// Returns an error when called during tests to prevent accidental real API calls.
/// Use `GitHubClient::with_base_url` in tests instead.
fn get_gh_token() -> Result<String> {
    #[cfg(test)]
    return Err(GitHubError::TokenError(
        "get_gh_token should not be called in tests. Use GitHubClient::with_base_url instead."
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
    use super::parse_repository_url;
    use crate::commands::gh::issue_agent::models::IssueTemplate;
    use crate::infra::github::mock::{GitHubMockServer, RemoteSubIssue};
    use indoc::indoc;
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
                body: Some(
                    indoc! {"
                        ## Description

                        Describe the bug"}
                    .to_string(),
                ),
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

    mod parse_repository_url_tests {
        use super::*;

        #[rstest]
        #[case::valid_url(
            "https://api.github.com/repos/octocat/hello-world",
            ("octocat", "hello-world")
        )]
        #[case::trailing_slash(
            "https://api.github.com/repos/owner/repo/",
            ("unknown", "unknown")
        )]
        #[case::empty_string("", ("unknown", "unknown"))]
        #[case::single_segment("foobar", ("unknown", "unknown"))]
        #[case::slash_only("/", ("unknown", "unknown"))]
        #[case::different_base_url(
            "https://example.com/some/path/my-org/my-repo",
            ("my-org", "my-repo")
        )]
        fn test_parse_repository_url(#[case] input: &str, #[case] expected: (&str, &str)) {
            let (owner, repo) = parse_repository_url(input);
            assert_eq!(owner, expected.0);
            assert_eq!(repo, expected.1);
        }
    }

    mod get_sub_issues_tests {
        use super::*;

        #[rstest]
        #[tokio::test]
        async fn test_returns_empty_when_no_sub_issues() {
            let mock = GitHubMockServer::start().await;
            mock.repo("owner", "repo").sub_issues_empty(1).await;

            let client = mock.client();
            let result = client.get_sub_issues("owner", "repo", 1).await;

            assert!(result.is_ok());
            assert!(result.unwrap().is_empty());
        }

        #[rstest]
        #[tokio::test]
        async fn test_returns_single_sub_issue() {
            let mock = GitHubMockServer::start().await;
            mock.repo("owner", "repo")
                .sub_issues(
                    1,
                    &[RemoteSubIssue {
                        id: 100,
                        number: 5,
                        owner: "owner",
                        repo: "repo",
                    }],
                )
                .await;

            let client = mock.client();
            let result = client.get_sub_issues("owner", "repo", 1).await;

            assert!(result.is_ok());
            let subs = result.unwrap();
            assert_eq!(subs.len(), 1);
            assert_eq!(subs[0].id, 100);
            assert_eq!(subs[0].number, 5);
            assert_eq!(subs[0].owner, "owner");
            assert_eq!(subs[0].repo, "repo");
        }

        #[rstest]
        #[tokio::test]
        async fn test_returns_multiple_sub_issues() {
            let mock = GitHubMockServer::start().await;
            mock.repo("owner", "repo")
                .sub_issues(
                    1,
                    &[
                        RemoteSubIssue {
                            id: 100,
                            number: 5,
                            owner: "owner",
                            repo: "repo",
                        },
                        RemoteSubIssue {
                            id: 200,
                            number: 10,
                            owner: "other-org",
                            repo: "other-repo",
                        },
                    ],
                )
                .await;

            let client = mock.client();
            let result = client.get_sub_issues("owner", "repo", 1).await;

            assert!(result.is_ok());
            let subs = result.unwrap();
            assert_eq!(subs.len(), 2);
            assert_eq!(subs[0].id, 100);
            assert_eq!(subs[0].number, 5);
            assert_eq!(subs[1].id, 200);
            assert_eq!(subs[1].number, 10);
            assert_eq!(subs[1].owner, "other-org");
            assert_eq!(subs[1].repo, "other-repo");
        }
    }

    mod add_sub_issue_tests {
        use super::*;

        #[rstest]
        #[tokio::test]
        async fn test_add_sub_issue_success() {
            let mock = GitHubMockServer::start().await;
            mock.repo("owner", "repo").add_sub_issue(1).await;

            let client = mock.client();
            let result = client.add_sub_issue("owner", "repo", 1, 999).await;

            assert!(result.is_ok());
        }
    }

    mod remove_sub_issue_tests {
        use super::*;

        #[rstest]
        #[tokio::test]
        async fn test_remove_sub_issue_success() {
            let mock = GitHubMockServer::start().await;
            mock.repo("owner", "repo").remove_sub_issue(1).await;

            let client = mock.client();
            let result = client.remove_sub_issue("owner", "repo", 1, 999).await;

            assert!(result.is_ok());
        }
    }

    mod get_issue_id_tests {
        use super::*;

        #[rstest]
        #[tokio::test]
        async fn test_returns_issue_id() {
            let mock = GitHubMockServer::start().await;
            mock.repo("owner", "repo").get_issue_id(42, 123456).await;

            let client = mock.client();
            let result = client.get_issue_id("owner", "repo", 42).await;

            assert!(result.is_ok());
            assert_eq!(result.unwrap(), 123456);
        }

        #[rstest]
        #[tokio::test]
        async fn test_returns_large_issue_id() {
            let mock = GitHubMockServer::start().await;
            mock.repo("owner", "repo")
                .get_issue_id(1, 9_999_999_999)
                .await;

            let client = mock.client();
            let result = client.get_issue_id("owner", "repo", 1).await;

            assert!(result.is_ok());
            assert_eq!(result.unwrap(), 9_999_999_999);
        }
    }
}
