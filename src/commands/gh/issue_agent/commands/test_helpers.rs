//! Test helpers shared across issue-agent command tests.

use crate::commands::gh::issue_agent::models::IssueMetadata;
use crate::commands::gh::issue_agent::storage::IssueStorage;
use crate::infra::github::OctocrabClient;
use rstest::fixture;
use serde_json::json;
use std::fs;
use std::path::Path;
use tempfile::TempDir;
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Create a temporary directory for test storage.
#[fixture]
pub fn test_dir() -> TempDir {
    tempfile::tempdir().unwrap()
}

/// Create metadata JSON using IssueMetadata serialization.
pub fn create_metadata_json(number: i64, title: &str, updated_at: &str, labels: &[&str]) -> String {
    let metadata = IssueMetadata {
        number,
        title: title.to_string(),
        state: "OPEN".to_string(),
        labels: labels.iter().map(|s| s.to_string()).collect(),
        assignees: vec![],
        milestone: None,
        author: "testuser".to_string(),
        created_at: "2024-01-01T00:00:00+00:00".to_string(),
        updated_at: updated_at.to_string(),
    };
    serde_json::to_string(&metadata).unwrap()
}

/// Create a comment file content with metadata headers.
pub fn create_comment_file(
    author: &str,
    created_at: &str,
    id: &str,
    database_id: u64,
    body: &str,
) -> String {
    format!(
        "<!-- author: {} -->\n<!-- createdAt: {} -->\n<!-- id: {} -->\n<!-- databaseId: {} -->\n\n{}",
        author, created_at, id, database_id, body
    )
}

/// Write a local comment file to the storage directory.
pub fn setup_local_comment(dir: &Path, filename: &str, content: &str) {
    let comments_dir = dir.join("comments");
    fs::create_dir_all(&comments_dir).unwrap();
    fs::write(comments_dir.join(filename), content).unwrap();
}

// Default timestamps for tests
pub const DEFAULT_TS: &str = "2024-01-02T00:00:00+00:00";
#[allow(dead_code)]
pub const OLD_TS: &str = "2024-01-01T00:00:00+00:00";

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
        "labels": [mock_label("bug")],
        "assignees": [],
        "author_association": "OWNER",
        "milestone": null,
        "locked": false,
        "comments": 0,
        "created_at": "2024-01-01T00:00:00Z",
        "updated_at": updated_at
    })
}

/// Macro to generate builder setter methods.
macro_rules! builder_setters {
    ($($field:ident: $ty:ty),* $(,)?) => {
        $(
            pub fn $field(mut self, v: $ty) -> Self {
                self.$field = v;
                self
            }
        )*
    };
}

/// Test fixture builder with sensible defaults.
///
/// Use this to set up GitHubMockServer and local storage for integration tests.
/// Only specify the fields you want to change from defaults.
///
/// # Example
/// ```ignore
/// let (mock, storage) = TestSetup::new(test_dir.path())
///     .remote_title("Old Title")
///     .local_title("New Title")
///     .build()
///     .await;
/// let client = mock.client();
/// ```
#[allow(dead_code)]
pub struct TestSetup<'a> {
    dir: &'a Path,
    // Remote state (what GitHub API returns)
    pub remote_title: &'a str,
    pub remote_body: &'a str,
    pub remote_ts: &'a str,
    pub remote_comments: Vec<RemoteComment<'a>>,
    // Local state (what's in the storage directory)
    pub local_title: &'a str,
    pub local_body: &'a str,
    pub local_labels: Vec<&'a str>,
    pub local_ts: &'a str,
    // Mock client config
    pub current_user: &'a str,
}

/// Remote comment definition for TestSetup.
#[derive(Clone)]
pub struct RemoteComment<'a> {
    pub id: &'a str,
    pub database_id: i64,
    pub author: &'a str,
    pub body: &'a str,
}

impl<'a> TestSetup<'a> {
    pub fn new(dir: &'a Path) -> Self {
        Self {
            dir,
            remote_title: "Title",
            remote_body: "Body",
            remote_ts: DEFAULT_TS,
            remote_comments: vec![],
            local_title: "Title",
            local_body: "Body",
            local_labels: vec!["bug"],
            local_ts: DEFAULT_TS,
            current_user: "testuser",
        }
    }

    builder_setters! {
        remote_title: &'a str,
        remote_body: &'a str,
        remote_ts: &'a str,
        remote_comments: Vec<RemoteComment<'a>>,
        local_title: &'a str,
        local_body: &'a str,
        local_labels: Vec<&'a str>,
        local_ts: &'a str,
        current_user: &'a str,
    }

    /// Build the GitHubMockServer and IssueStorage.
    pub async fn build(self) -> (GitHubMockServer, IssueStorage) {
        let mock = GitHubMockServer::start().await;

        // Set up remote issue mock
        mock.mock_get_issue_with(
            "owner",
            "repo",
            123,
            self.remote_title,
            self.remote_body,
            self.remote_ts,
        )
        .await;

        // Set up remote comments mock
        mock.mock_get_comments_graphql_with("owner", "repo", 123, &self.remote_comments)
            .await;

        // Set up current user mock
        mock.mock_get_current_user(self.current_user).await;

        // Set up local storage
        fs::create_dir_all(self.dir).unwrap();
        fs::write(self.dir.join("issue.md"), format!("{}\n", self.local_body)).unwrap();
        fs::write(
            self.dir.join("metadata.json"),
            create_metadata_json(123, self.local_title, self.local_ts, &self.local_labels),
        )
        .unwrap();

        let storage = IssueStorage::from_dir(self.dir);
        (mock, storage)
    }
}

/// Create a RemoteComment for use with TestSetup.
#[allow(dead_code)]
pub fn make_remote_comment<'a>(
    author: &'a str,
    id: &'a str,
    database_id: i64,
    body: &'a str,
) -> RemoteComment<'a> {
    RemoteComment {
        id,
        database_id,
        author,
        body,
    }
}

/// wiremock-based GitHub mock server for testing.
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
        // Note: add_labels returns a list of labels, but we don't use the response
        // so we just return an empty success response
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
        // GitHub returns 204 No Content, but octocrab expects some JSON response
        // so we return an empty object
        Mock::given(method("DELETE"))
            .and(path_regex(format!(
                r"/repos/{owner}/{repo}/issues/comments/\d+"
            )))
            .respond_with(ResponseTemplate::new(204).set_body_json(json!({})))
            .mount(&self.server)
            .await;
    }

    /// Mock GET /user for current user
    pub async fn mock_get_current_user(&self, login: &str) {
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_user(login)))
            .mount(&self.server)
            .await;
    }
}
