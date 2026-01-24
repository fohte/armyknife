//! wiremock-based GitHub mock server for testing.
//!
//! Provides `GitHubMockServer` for HTTP-level mocking of GitHub API calls.
//! This is used across all tests that need to interact with GitHub APIs.
//!
//! # Usage
//!
//! Use the builder pattern via `mock.repo(owner, repo)` for a fluent API:
//!
//! ```ignore
//! let mock = GitHubMockServer::start().await;
//! let ctx = mock.repo("owner", "repo");
//!
//! // Issue operations
//! ctx.issue(123).get().await;
//! ctx.issue(123).title("Custom").body("Body").get().await;
//! ctx.issue(123).get_not_found().await;
//! ctx.issue(123).update().await;
//! ctx.issue(123).add_labels().await;
//! ctx.issue(123).remove_label("bug").await;
//!
//! // Comment operations
//! ctx.issue(123).create_comment().await;
//! ctx.comment().update().await;
//! ctx.comment().delete().await;
//! ctx.graphql_comments(&[]).await;
//!
//! // Repository operations
//! ctx.repo_info().get().await;
//! ctx.repo_info().private(true).default_branch("develop").get().await;
//!
//! // Pull request operations
//! ctx.pull_request(1).create().await;
//!
//! // User operations (server-level, not repo-scoped)
//! mock.current_user("testuser").await;
//! ```

use serde_json::json;
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::client::OctocrabClient;

/// Create a mock user JSON object for octocrab Author model.
fn mock_user(login: &str) -> serde_json::Value {
    json!({
        "login": login,
        "id": 1,
        "node_id": "U_test",
        "avatar_url": "https://avatars.githubusercontent.com/u/1",
        "gravatar_id": "",
        "url": format!("https://api.github.com/users/{}", login),
        "html_url": format!("https://github.com/{}", login),
        "followers_url": format!("https://api.github.com/users/{}/followers", login),
        "following_url": format!("https://api.github.com/users/{}/following{{/other_user}}", login),
        "gists_url": format!("https://api.github.com/users/{}/gists{{/gist_id}}", login),
        "starred_url": format!("https://api.github.com/users/{}/starred{{/owner}}{{/repo}}", login),
        "subscriptions_url": format!("https://api.github.com/users/{}/subscriptions", login),
        "organizations_url": format!("https://api.github.com/users/{}/orgs", login),
        "repos_url": format!("https://api.github.com/users/{}/repos", login),
        "events_url": format!("https://api.github.com/users/{}/events{{/privacy}}", login),
        "received_events_url": format!("https://api.github.com/users/{}/received_events", login),
        "type": "User",
        "site_admin": false
    })
}

/// Create a mock label JSON object for octocrab Label model.
fn mock_label(name: &str) -> serde_json::Value {
    json!({
        "id": 1,
        "node_id": "LA_test",
        "url": format!("https://api.github.com/repos/owner/repo/labels/{}", name),
        "name": name,
        "color": "d73a4a",
        "default": false
    })
}

/// Create a mock comment JSON object for octocrab Comment model.
fn mock_comment(
    owner: &str,
    repo: &str,
    issue_number: u64,
    comment_id: u64,
    node_id: &str,
    author: &str,
    body: &str,
) -> serde_json::Value {
    json!({
        "id": comment_id,
        "node_id": node_id,
        "url": format!("https://api.github.com/repos/{}/{}/issues/comments/{}", owner, repo, comment_id),
        "html_url": format!("https://github.com/{}/{}/issues/{}#issuecomment-{}", owner, repo, issue_number, comment_id),
        "body": body,
        "author_association": "OWNER",
        "user": mock_user(author),
        "created_at": "2024-01-02T00:00:00Z"
    })
}

/// Create a mock issue JSON object for octocrab Issue model.
fn mock_issue(
    owner: &str,
    repo: &str,
    issue_number: u64,
    title: &str,
    body: &str,
    updated_at: &str,
) -> serde_json::Value {
    mock_issue_with_labels(owner, repo, issue_number, title, body, updated_at, &["bug"])
}

fn mock_issue_with_labels(
    owner: &str,
    repo: &str,
    issue_number: u64,
    title: &str,
    body: &str,
    updated_at: &str,
    labels: &[&str],
) -> serde_json::Value {
    let mock_labels: Vec<_> = labels.iter().map(|l| mock_label(l)).collect();
    json!({
        "id": 1,
        "node_id": "I_test",
        "url": format!("https://api.github.com/repos/{}/{}/issues/{}", owner, repo, issue_number),
        "repository_url": format!("https://api.github.com/repos/{}/{}", owner, repo),
        "labels_url": format!("https://api.github.com/repos/{}/{}/issues/{}/labels{{/name}}", owner, repo, issue_number),
        "comments_url": format!("https://api.github.com/repos/{}/{}/issues/{}/comments", owner, repo, issue_number),
        "events_url": format!("https://api.github.com/repos/{}/{}/issues/{}/events", owner, repo, issue_number),
        "html_url": format!("https://github.com/{}/{}/issues/{}", owner, repo, issue_number),
        "number": issue_number,
        "state": "open",
        "title": title,
        "body": body,
        "user": mock_user("testuser"),
        "labels": mock_labels,
        "assignees": [],
        "author_association": "OWNER",
        "milestone": null,
        "locked": false,
        "comments": 0,
        "created_at": "2024-01-01T00:00:00Z",
        "updated_at": updated_at
    })
}

/// Create a mock repository JSON object for octocrab Repository model.
fn mock_repository(
    owner: &str,
    repo: &str,
    is_private: bool,
    default_branch: &str,
) -> serde_json::Value {
    json!({
        "id": 1,
        "node_id": "R_test",
        "name": repo,
        "full_name": format!("{}/{}", owner, repo),
        "private": is_private,
        "owner": mock_user(owner),
        "html_url": format!("https://github.com/{}/{}", owner, repo),
        "description": "Test repository",
        "fork": false,
        "url": format!("https://api.github.com/repos/{}/{}", owner, repo),
        "created_at": "2024-01-01T00:00:00Z",
        "updated_at": "2024-01-01T00:00:00Z",
        "pushed_at": "2024-01-01T00:00:00Z",
        "default_branch": default_branch
    })
}

/// Create a mock pull request JSON object for octocrab PullRequest model.
fn mock_pull_request(
    owner: &str,
    repo: &str,
    pr_number: u64,
    title: &str,
    body: &str,
    head: &str,
) -> serde_json::Value {
    json!({
        "id": 1,
        "node_id": "PR_test",
        "number": pr_number,
        "state": "open",
        "locked": false,
        "title": title,
        "body": body,
        "user": mock_user("testuser"),
        "url": format!("https://api.github.com/repos/{}/{}/pulls/{}", owner, repo, pr_number),
        "html_url": format!("https://github.com/{}/{}/pull/{}", owner, repo, pr_number),
        "diff_url": format!("https://github.com/{}/{}/pull/{}.diff", owner, repo, pr_number),
        "patch_url": format!("https://github.com/{}/{}/pull/{}.patch", owner, repo, pr_number),
        "issue_url": format!("https://api.github.com/repos/{}/{}/issues/{}", owner, repo, pr_number),
        "commits_url": format!("https://api.github.com/repos/{}/{}/pulls/{}/commits", owner, repo, pr_number),
        "review_comments_url": format!("https://api.github.com/repos/{}/{}/pulls/{}/comments", owner, repo, pr_number),
        "review_comment_url": format!("https://api.github.com/repos/{}/{}/pulls/comments{{/number}}", owner, repo),
        "comments_url": format!("https://api.github.com/repos/{}/{}/issues/{}/comments", owner, repo, pr_number),
        "statuses_url": format!("https://api.github.com/repos/{}/{}/statuses/abc123", owner, repo),
        "created_at": "2024-01-01T00:00:00Z",
        "updated_at": "2024-01-01T00:00:00Z",
        "head": {
            "label": format!("{}:{}", owner, head),
            "ref": head,
            "sha": "abc123"
        },
        "base": {
            "label": format!("{}:main", owner),
            "ref": "main",
            "sha": "def456"
        }
    })
}

/// Remote comment definition for test setup.
#[derive(Clone)]
pub struct RemoteComment<'a> {
    pub id: &'a str,
    pub database_id: i64,
    pub author: &'a str,
    pub body: &'a str,
}

/// wiremock-based GitHub mock server for testing.
///
/// This provides HTTP-level mocking for GitHub API endpoints, allowing tests
/// to verify actual HTTP requests rather than mocking at the trait level.
pub struct GitHubMockServer {
    server: MockServer,
}

impl GitHubMockServer {
    /// Start a new mock server.
    pub async fn start() -> Self {
        Self {
            server: MockServer::start().await,
        }
    }

    /// Get an OctocrabClient configured to use this mock server.
    pub fn client(&self) -> OctocrabClient {
        OctocrabClient::with_base_url(&self.server.uri(), "test-token").unwrap()
    }

    /// Create a repository context for building mocks.
    ///
    /// This is the preferred way to set up mocks. Example:
    /// ```ignore
    /// let ctx = mock.repo("owner", "repo");
    /// ctx.issue(123).title("Title").get().await;
    /// ```
    pub fn repo<'a>(&'a self, owner: &'a str, repo: &'a str) -> MockRepoContext<'a> {
        MockRepoContext {
            server: &self.server,
            owner,
            repo,
        }
    }

    /// Mock GET /user for current user.
    pub async fn current_user(&self, login: &str) {
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_user(login)))
            .mount(&self.server)
            .await;
    }
}

// ============ Builder Pattern API ============

/// Repository context for building mocks.
///
/// Created via `GitHubMockServer::repo()`. Provides builders for various
/// GitHub API endpoints scoped to a specific repository.
pub struct MockRepoContext<'a> {
    server: &'a MockServer,
    owner: &'a str,
    repo: &'a str,
}

impl<'a> MockRepoContext<'a> {
    /// Create an issue mock builder.
    pub fn issue(&self, number: u64) -> MockIssueBuilder<'_> {
        MockIssueBuilder {
            server: self.server,
            owner: self.owner,
            repo: self.repo,
            number,
            title: "Test Issue",
            body: "Test body",
            updated_at: "2024-01-02T00:00:00Z",
            labels: vec!["bug"],
        }
    }

    /// Create a comment mock builder (for update/delete operations).
    pub fn comment(&self) -> MockCommentBuilder<'_> {
        MockCommentBuilder {
            server: self.server,
            owner: self.owner,
            repo: self.repo,
        }
    }

    /// Mock GraphQL endpoint for fetching comments.
    pub async fn graphql_comments(&self, comments: &[RemoteComment<'_>]) {
        let nodes: Vec<serde_json::Value> = comments
            .iter()
            .map(|c| {
                json!({
                    "id": c.id,
                    "databaseId": c.database_id,
                    "author": {"login": c.author},
                    "createdAt": "2024-01-01T12:00:00Z",
                    "body": c.body
                })
            })
            .collect();

        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "repository": {
                        "issue": {
                            "comments": {
                                "nodes": nodes
                            }
                        }
                    }
                }
            })))
            .mount(self.server)
            .await;
    }

    /// Create a repository info mock builder.
    pub fn repo_info(&self) -> MockRepoInfoBuilder<'_> {
        MockRepoInfoBuilder {
            server: self.server,
            owner: self.owner,
            repo: self.repo,
            is_private: false,
            default_branch: "main",
        }
    }

    /// Create a pull request mock builder.
    pub fn pull_request(&self, number: u64) -> MockPullRequestBuilder<'_> {
        MockPullRequestBuilder {
            server: self.server,
            owner: self.owner,
            repo: self.repo,
            number,
        }
    }
}

/// Builder for mocking issue endpoints.
pub struct MockIssueBuilder<'a> {
    server: &'a MockServer,
    owner: &'a str,
    repo: &'a str,
    number: u64,
    title: &'a str,
    body: &'a str,
    updated_at: &'a str,
    labels: Vec<&'a str>,
}

impl<'a> MockIssueBuilder<'a> {
    /// Set the issue title.
    pub fn title(mut self, title: &'a str) -> Self {
        self.title = title;
        self
    }

    /// Set the issue body.
    pub fn body(mut self, body: &'a str) -> Self {
        self.body = body;
        self
    }

    /// Set the updated_at timestamp.
    pub fn updated_at(mut self, updated_at: &'a str) -> Self {
        self.updated_at = updated_at;
        self
    }

    /// Set the labels.
    pub fn labels(mut self, labels: Vec<&'a str>) -> Self {
        self.labels = labels;
        self
    }

    /// Mount mock for GET /repos/{owner}/{repo}/issues/{number} (success).
    pub async fn get(self) {
        let owner = self.owner;
        let repo = self.repo;
        let number = self.number;
        Mock::given(method("GET"))
            .and(path(format!("/repos/{owner}/{repo}/issues/{number}")))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(mock_issue_with_labels(
                    owner,
                    repo,
                    number,
                    self.title,
                    self.body,
                    self.updated_at,
                    &self.labels,
                )),
            )
            .mount(self.server)
            .await;
    }

    /// Mount mock for GET /repos/{owner}/{repo}/issues/{number} returning 404.
    pub async fn get_not_found(self) {
        let owner = self.owner;
        let repo = self.repo;
        let number = self.number;
        Mock::given(method("GET"))
            .and(path(format!("/repos/{owner}/{repo}/issues/{number}")))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({
                "message": "Not Found",
                "documentation_url": "https://docs.github.com/rest"
            })))
            .mount(self.server)
            .await;
    }

    /// Mount mock for PATCH /repos/{owner}/{repo}/issues/{number}.
    pub async fn update(self) {
        let owner = self.owner;
        let repo = self.repo;
        let number = self.number;
        Mock::given(method("PATCH"))
            .and(path(format!("/repos/{owner}/{repo}/issues/{number}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_issue(
                owner,
                repo,
                number,
                "Test Issue",
                "Updated body",
                "2024-01-03T00:00:00Z",
            )))
            .mount(self.server)
            .await;
    }

    /// Mount mock for POST /repos/{owner}/{repo}/issues/{number}/labels.
    pub async fn add_labels(self) {
        let owner = self.owner;
        let repo = self.repo;
        let number = self.number;
        Mock::given(method("POST"))
            .and(path(format!(
                "/repos/{owner}/{repo}/issues/{number}/labels"
            )))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!([mock_label("enhancement")])),
            )
            .mount(self.server)
            .await;
    }

    /// Mount mock for DELETE /repos/{owner}/{repo}/issues/{number}/labels/{label}.
    pub async fn remove_label(self, label: &str) {
        let owner = self.owner;
        let repo = self.repo;
        let number = self.number;
        Mock::given(method("DELETE"))
            .and(path(format!(
                "/repos/{owner}/{repo}/issues/{number}/labels/{label}"
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(self.server)
            .await;
    }

    /// Mount mock for POST /repos/{owner}/{repo}/issues/{number}/comments.
    pub async fn create_comment(self) {
        let owner = self.owner;
        let repo = self.repo;
        let number = self.number;
        Mock::given(method("POST"))
            .and(path(format!(
                "/repos/{owner}/{repo}/issues/{number}/comments"
            )))
            .respond_with(ResponseTemplate::new(201).set_body_json(mock_comment(
                owner,
                repo,
                number,
                99999,
                "IC_new_comment",
                "testuser",
                "New comment",
            )))
            .mount(self.server)
            .await;
    }
}

/// Builder for mocking comment update/delete endpoints.
pub struct MockCommentBuilder<'a> {
    server: &'a MockServer,
    owner: &'a str,
    repo: &'a str,
}

impl<'a> MockCommentBuilder<'a> {
    /// Mount mock for PATCH /repos/{owner}/{repo}/issues/comments/{id}.
    pub async fn update(self) {
        let owner = self.owner;
        let repo = self.repo;
        Mock::given(method("PATCH"))
            .and(path_regex(format!(
                r"/repos/{owner}/{repo}/issues/comments/\d+"
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_comment(
                owner,
                repo,
                1,
                12345,
                "IC_updated",
                "testuser",
                "Updated comment",
            )))
            .mount(self.server)
            .await;
    }

    /// Mount mock for DELETE /repos/{owner}/{repo}/issues/comments/{id}.
    pub async fn delete(self) {
        let owner = self.owner;
        let repo = self.repo;
        Mock::given(method("DELETE"))
            .and(path_regex(format!(
                r"/repos/{owner}/{repo}/issues/comments/\d+"
            )))
            .respond_with(ResponseTemplate::new(204))
            .mount(self.server)
            .await;
    }
}

/// Builder for mocking repository info endpoints.
pub struct MockRepoInfoBuilder<'a> {
    server: &'a MockServer,
    owner: &'a str,
    repo: &'a str,
    is_private: bool,
    default_branch: &'a str,
}

impl<'a> MockRepoInfoBuilder<'a> {
    /// Set whether the repository is private.
    pub fn private(mut self, is_private: bool) -> Self {
        self.is_private = is_private;
        self
    }

    /// Set the default branch.
    pub fn default_branch(mut self, branch: &'a str) -> Self {
        self.default_branch = branch;
        self
    }

    /// Mount mock for GET /repos/{owner}/{repo}.
    pub async fn get(self) {
        let owner = self.owner;
        let repo = self.repo;
        Mock::given(method("GET"))
            .and(path(format!("/repos/{owner}/{repo}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_repository(
                owner,
                repo,
                self.is_private,
                self.default_branch,
            )))
            .mount(self.server)
            .await;
    }
}

/// Builder for mocking pull request endpoints.
pub struct MockPullRequestBuilder<'a> {
    server: &'a MockServer,
    owner: &'a str,
    repo: &'a str,
    number: u64,
}

impl<'a> MockPullRequestBuilder<'a> {
    /// Mount mock for POST /repos/{owner}/{repo}/pulls.
    pub async fn create(self) {
        let owner = self.owner;
        let repo = self.repo;
        Mock::given(method("POST"))
            .and(path(format!("/repos/{owner}/{repo}/pulls")))
            .respond_with(ResponseTemplate::new(201).set_body_json(mock_pull_request(
                owner,
                repo,
                self.number,
                "Test PR",
                "Test body",
                "feature",
            )))
            .mount(self.server)
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::github::RepoClient;

    #[tokio::test]
    async fn mock_server_returns_repository_info() {
        let mock = GitHubMockServer::start().await;
        mock.repo("owner", "repo").repo_info().get().await;

        let client = mock.client();
        let is_private = client.is_repo_private("owner", "repo").await.unwrap();
        assert!(!is_private);
    }

    #[tokio::test]
    async fn mock_server_returns_configured_default_branch() {
        let mock = GitHubMockServer::start().await;
        mock.repo("owner", "repo")
            .repo_info()
            .default_branch("develop")
            .get()
            .await;

        let client = mock.client();
        let branch = client.get_default_branch("owner", "repo").await.unwrap();
        assert_eq!(branch, "develop");
    }
}
