//! wiremock-based GitHub mock server for testing.
//!
//! Provides `GitHubMockServer` for HTTP-level mocking of GitHub API calls.
//! This is used across all tests that need to interact with GitHub APIs.

use serde_json::json;
use wiremock::matchers::{method, path, path_regex, query_param};
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

    // ============ Issue Operations ============

    /// Mock GET /repos/{owner}/{repo}/issues/{issue_number}
    pub async fn mock_get_issue(&self, owner: &str, repo: &str, issue_number: u64) {
        Mock::given(method("GET"))
            .and(path(format!("/repos/{owner}/{repo}/issues/{issue_number}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_issue(
                owner,
                repo,
                issue_number,
                "Test Issue",
                "Test body",
                "2024-01-02T00:00:00Z",
            )))
            .mount(&self.server)
            .await;
    }

    /// Mock GET /repos/{owner}/{repo}/issues/{issue_number} with custom data
    pub async fn mock_get_issue_with(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        title: &str,
        body: &str,
        updated_at: &str,
    ) {
        Mock::given(method("GET"))
            .and(path(format!("/repos/{owner}/{repo}/issues/{issue_number}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_issue(
                owner,
                repo,
                issue_number,
                title,
                body,
                updated_at,
            )))
            .mount(&self.server)
            .await;
    }

    /// Mock GET /repos/{owner}/{repo}/issues/{issue_number} with custom labels
    pub async fn mock_get_issue_with_labels(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        title: &str,
        body: &str,
        updated_at: &str,
        labels: &[&str],
    ) {
        Mock::given(method("GET"))
            .and(path(format!("/repos/{owner}/{repo}/issues/{issue_number}")))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(mock_issue_with_labels(
                    owner,
                    repo,
                    issue_number,
                    title,
                    body,
                    updated_at,
                    labels,
                )),
            )
            .mount(&self.server)
            .await;
    }

    /// Mock GET /repos/{owner}/{repo}/issues/{issue_number} returning 404
    pub async fn mock_get_issue_not_found(&self, owner: &str, repo: &str, issue_number: u64) {
        Mock::given(method("GET"))
            .and(path(format!("/repos/{owner}/{repo}/issues/{issue_number}")))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({
                "message": "Not Found",
                "documentation_url": "https://docs.github.com/rest"
            })))
            .mount(&self.server)
            .await;
    }

    /// Mock PATCH /repos/{owner}/{repo}/issues/{issue_number} for body update
    pub async fn mock_update_issue(&self, owner: &str, repo: &str, issue_number: u64) {
        Mock::given(method("PATCH"))
            .and(path(format!("/repos/{owner}/{repo}/issues/{issue_number}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_issue(
                owner,
                repo,
                issue_number,
                "Test Issue",
                "Updated body",
                "2024-01-03T00:00:00Z",
            )))
            .mount(&self.server)
            .await;
    }

    /// Mock POST /repos/{owner}/{repo}/issues/{issue_number}/labels
    pub async fn mock_add_labels(&self, owner: &str, repo: &str, issue_number: u64) {
        Mock::given(method("POST"))
            .and(path(format!(
                "/repos/{owner}/{repo}/issues/{issue_number}/labels"
            )))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!([mock_label("enhancement")])),
            )
            .mount(&self.server)
            .await;
    }

    /// Mock DELETE /repos/{owner}/{repo}/issues/{issue_number}/labels/{label}
    pub async fn mock_remove_label(&self, owner: &str, repo: &str, issue_number: u64, label: &str) {
        Mock::given(method("DELETE"))
            .and(path(format!(
                "/repos/{owner}/{repo}/issues/{issue_number}/labels/{label}"
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&self.server)
            .await;
    }

    // ============ Comment Operations ============

    /// Mock GraphQL endpoint for get_comments
    pub async fn mock_get_comments_graphql(&self, _owner: &str, _repo: &str, _issue_number: u64) {
        self.mock_get_comments_graphql_with(_owner, _repo, _issue_number, &[])
            .await;
    }

    /// Mock GraphQL endpoint for get_comments with custom comments
    pub async fn mock_get_comments_graphql_with(
        &self,
        _owner: &str,
        _repo: &str,
        _issue_number: u64,
        comments: &[RemoteComment<'_>],
    ) {
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
            .mount(&self.server)
            .await;
    }

    /// Mock POST /repos/{owner}/{repo}/issues/{issue_number}/comments
    pub async fn mock_create_comment(&self, owner: &str, repo: &str, issue_number: u64) {
        Mock::given(method("POST"))
            .and(path(format!(
                "/repos/{owner}/{repo}/issues/{issue_number}/comments"
            )))
            .respond_with(ResponseTemplate::new(201).set_body_json(mock_comment(
                owner,
                repo,
                issue_number,
                99999,
                "IC_new_comment",
                "testuser",
                "New comment",
            )))
            .mount(&self.server)
            .await;
    }

    /// Mock PATCH /repos/{owner}/{repo}/issues/comments/{comment_id}
    pub async fn mock_update_comment(&self, owner: &str, repo: &str, _comment_id: u64) {
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
            .mount(&self.server)
            .await;
    }

    /// Mock DELETE /repos/{owner}/{repo}/issues/comments/{comment_id}
    pub async fn mock_delete_comment(&self, owner: &str, repo: &str, _comment_id: u64) {
        Mock::given(method("DELETE"))
            .and(path_regex(format!(
                r"/repos/{owner}/{repo}/issues/comments/\d+"
            )))
            .respond_with(ResponseTemplate::new(204))
            .mount(&self.server)
            .await;
    }

    // ============ User Operations ============

    /// Mock GET /user for current user
    pub async fn mock_get_current_user(&self, login: &str) {
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_user(login)))
            .mount(&self.server)
            .await;
    }

    // ============ Repository Operations ============

    /// Mock GET /repos/{owner}/{repo} for repository info
    pub async fn mock_get_repo(&self, owner: &str, repo: &str, is_private: bool) {
        self.mock_get_repo_with_branch(owner, repo, is_private, "main")
            .await;
    }

    /// Mock GET /repos/{owner}/{repo} with custom default branch
    pub async fn mock_get_repo_with_branch(
        &self,
        owner: &str,
        repo: &str,
        is_private: bool,
        default_branch: &str,
    ) {
        Mock::given(method("GET"))
            .and(path(format!("/repos/{owner}/{repo}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_repository(
                owner,
                repo,
                is_private,
                default_branch,
            )))
            .mount(&self.server)
            .await;
    }

    // ============ Pull Request Operations ============

    /// Mock POST /repos/{owner}/{repo}/pulls for PR creation.
    pub async fn mock_create_pull_request(&self, owner: &str, repo: &str) {
        self.mock_create_pull_request_with(owner, repo, 1, "Test PR", "Test body", "feature")
            .await;
    }

    /// Mock POST /repos/{owner}/{repo}/pulls with custom data.
    pub async fn mock_create_pull_request_with(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        title: &str,
        body: &str,
        head: &str,
    ) {
        Mock::given(method("POST"))
            .and(path(format!("/repos/{owner}/{repo}/pulls")))
            .respond_with(
                ResponseTemplate::new(201)
                    .set_body_json(mock_pull_request(owner, repo, pr_number, title, body, head)),
            )
            .mount(&self.server)
            .await;
    }

    /// Mock GET /repos/{owner}/{repo}/pulls for listing PRs (returns empty list).
    pub async fn mock_list_pull_requests_empty(&self, owner: &str, repo: &str) {
        Mock::given(method("GET"))
            .and(path(format!("/repos/{owner}/{repo}/pulls")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&self.server)
            .await;
    }

    /// Mock GET /repos/{owner}/{repo}/pulls for listing PRs with existing PR.
    pub async fn mock_list_pull_requests_with(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        title: &str,
        body: &str,
        head: &str,
    ) {
        Mock::given(method("GET"))
            .and(path(format!("/repos/{owner}/{repo}/pulls")))
            .and(query_param("head", format!("{owner}:{head}")))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!([mock_pull_request(
                    owner, repo, pr_number, title, body, head
                )])),
            )
            .mount(&self.server)
            .await;
    }

    /// Mock PATCH /repos/{owner}/{repo}/pulls/{pr_number} for updating PR.
    pub async fn mock_update_pull_request(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        title: &str,
        body: &str,
        head: &str,
    ) {
        Mock::given(method("PATCH"))
            .and(path(format!("/repos/{owner}/{repo}/pulls/{pr_number}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(mock_pull_request(owner, repo, pr_number, title, body, head)),
            )
            .mount(&self.server)
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
        mock.mock_get_repo("owner", "repo", false).await;

        let client = mock.client();
        let is_private = client.is_repo_private("owner", "repo").await.unwrap();
        assert!(!is_private);
    }

    #[tokio::test]
    async fn mock_server_returns_configured_default_branch() {
        let mock = GitHubMockServer::start().await;
        mock.mock_get_repo_with_branch("owner", "repo", false, "develop")
            .await;

        let client = mock.client();
        let branch = client.get_default_branch("owner", "repo").await.unwrap();
        assert_eq!(branch, "develop");
    }
}
