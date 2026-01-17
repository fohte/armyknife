//! Mock implementations for testing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::comment::CommentClient;
use super::error::{GitHubError, Result};
use super::issue::IssueClient;
use super::pr::{CreatePrParams, PrClient, PrInfo};
use super::repo::RepoClient;
use crate::gh::issue_agent::models::{Author, Comment, Issue};

/// Mock implementation for testing.
#[derive(Clone)]
pub struct MockGitHubClient {
    /// Map of "owner/repo" -> is_private
    pub private_repos: HashMap<String, bool>,
    /// Map of "owner/repo" -> default_branch
    pub default_branches: HashMap<String, String>,
    /// Result URL for PR creation (None = error)
    pub pr_create_result: Option<String>,
    /// Map of "owner/repo/branch" -> PrInfo
    pub branch_prs: HashMap<String, PrInfo>,
    /// Track created PRs for assertions
    pub created_prs: Arc<Mutex<Vec<CreatePrParams>>>,
    /// Track browser opens for assertions
    pub opened_urls: Arc<Mutex<Vec<String>>>,
    /// Map of "owner/repo/issue_number" -> Issue
    pub issues: HashMap<String, Issue>,
    /// Map of "owner/repo/issue_number" -> Vec<Comment>
    pub comments: HashMap<String, Vec<Comment>>,
    /// Track updated issue bodies for assertions
    pub updated_issue_bodies: Arc<Mutex<Vec<UpdateIssueBodyParams>>>,
    /// Track updated issue titles for assertions
    pub updated_issue_titles: Arc<Mutex<Vec<UpdateIssueTitleParams>>>,
    /// Track added labels for assertions
    pub added_labels: Arc<Mutex<Vec<AddLabelsParams>>>,
    /// Track removed labels for assertions
    pub removed_labels: Arc<Mutex<Vec<RemoveLabelParams>>>,
    /// Track updated comments for assertions
    pub updated_comments: Arc<Mutex<Vec<UpdateCommentParams>>>,
    /// Track created comments for assertions
    pub created_comments: Arc<Mutex<Vec<CreateCommentParams>>>,
    /// Current user login for testing
    pub current_user: Option<String>,
}

/// Parameters for tracking update_issue_body calls.
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateIssueBodyParams {
    pub owner: String,
    pub repo: String,
    pub issue_number: u64,
    pub body: String,
}

/// Parameters for tracking update_issue_title calls.
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateIssueTitleParams {
    pub owner: String,
    pub repo: String,
    pub issue_number: u64,
    pub title: String,
}

/// Parameters for tracking add_labels calls.
#[derive(Debug, Clone, PartialEq)]
pub struct AddLabelsParams {
    pub owner: String,
    pub repo: String,
    pub issue_number: u64,
    pub labels: Vec<String>,
}

/// Parameters for tracking remove_label calls.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoveLabelParams {
    pub owner: String,
    pub repo: String,
    pub issue_number: u64,
    pub label: String,
}

/// Parameters for tracking update_comment calls.
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateCommentParams {
    pub owner: String,
    pub repo: String,
    pub comment_id: u64,
    pub body: String,
}

/// Parameters for tracking create_comment calls.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateCommentParams {
    pub owner: String,
    pub repo: String,
    pub issue_number: u64,
    pub body: String,
}

impl MockGitHubClient {
    pub fn new() -> Self {
        Self {
            private_repos: HashMap::new(),
            default_branches: HashMap::new(),
            pr_create_result: Some("https://github.com/owner/repo/pull/1".to_string()),
            branch_prs: HashMap::new(),
            created_prs: Arc::new(Mutex::new(Vec::new())),
            opened_urls: Arc::new(Mutex::new(Vec::new())),
            issues: HashMap::new(),
            comments: HashMap::new(),
            updated_issue_bodies: Arc::new(Mutex::new(Vec::new())),
            updated_issue_titles: Arc::new(Mutex::new(Vec::new())),
            added_labels: Arc::new(Mutex::new(Vec::new())),
            removed_labels: Arc::new(Mutex::new(Vec::new())),
            updated_comments: Arc::new(Mutex::new(Vec::new())),
            created_comments: Arc::new(Mutex::new(Vec::new())),
            current_user: None,
        }
    }

    pub fn with_private(mut self, owner: &str, repo: &str, is_private: bool) -> Self {
        self.private_repos
            .insert(format!("{owner}/{repo}"), is_private);
        self
    }

    pub fn with_default_branch(mut self, owner: &str, repo: &str, branch: &str) -> Self {
        self.default_branches
            .insert(format!("{owner}/{repo}"), branch.to_string());
        self
    }

    #[allow(dead_code)]
    pub fn with_pr_result(mut self, result: Option<String>) -> Self {
        self.pr_create_result = result;
        self
    }

    pub fn with_branch_pr(
        mut self,
        owner: &str,
        repo: &str,
        branch: &str,
        pr_info: PrInfo,
    ) -> Self {
        self.branch_prs
            .insert(format!("{owner}/{repo}/{branch}"), pr_info);
        self
    }

    /// Add an issue to the mock client.
    #[allow(dead_code)]
    pub fn with_issue(mut self, owner: &str, repo: &str, issue: Issue) -> Self {
        let key = format!("{owner}/{repo}/{}", issue.number);
        self.issues.insert(key, issue);
        self
    }

    /// Add comments to an issue in the mock client.
    #[allow(dead_code)]
    pub fn with_comments(
        mut self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        comments: Vec<Comment>,
    ) -> Self {
        let key = format!("{owner}/{repo}/{issue_number}");
        self.comments.insert(key, comments);
        self
    }

    /// Set the current user for the mock client.
    #[allow(dead_code)]
    pub fn with_current_user(mut self, login: &str) -> Self {
        self.current_user = Some(login.to_string());
        self
    }
}

#[async_trait::async_trait]
impl RepoClient for MockGitHubClient {
    async fn is_repo_private(&self, owner: &str, repo: &str) -> Result<bool> {
        let key = format!("{owner}/{repo}");
        Ok(self.private_repos.get(&key).copied().unwrap_or(true))
    }

    async fn get_default_branch(&self, owner: &str, repo: &str) -> Result<String> {
        let key = format!("{owner}/{repo}");
        Ok(self
            .default_branches
            .get(&key)
            .cloned()
            .unwrap_or_else(|| "main".to_string()))
    }
}

#[async_trait::async_trait]
impl PrClient for MockGitHubClient {
    async fn create_pull_request(&self, params: CreatePrParams) -> Result<String> {
        self.created_prs.lock().unwrap().push(params);
        self.pr_create_result
            .clone()
            .ok_or_else(|| GitHubError::TokenError("Mock PR creation failed".to_string()))
    }

    async fn get_pr_for_branch(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<Option<PrInfo>> {
        let key = format!("{owner}/{repo}/{branch}");
        Ok(self.branch_prs.get(&key).cloned())
    }

    fn open_in_browser(&self, url: &str) {
        self.opened_urls.lock().unwrap().push(url.to_string());
    }
}

#[async_trait::async_trait]
impl IssueClient for MockGitHubClient {
    async fn get_issue(&self, owner: &str, repo: &str, issue_number: u64) -> Result<Issue> {
        let key = format!("{owner}/{repo}/{issue_number}");
        self.issues
            .get(&key)
            .cloned()
            .ok_or_else(|| GitHubError::TokenError(format!("Issue {key} not found in mock")))
    }

    async fn update_issue_body(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        body: &str,
    ) -> Result<()> {
        self.updated_issue_bodies
            .lock()
            .unwrap()
            .push(UpdateIssueBodyParams {
                owner: owner.to_string(),
                repo: repo.to_string(),
                issue_number,
                body: body.to_string(),
            });
        Ok(())
    }

    async fn update_issue_title(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        title: &str,
    ) -> Result<()> {
        self.updated_issue_titles
            .lock()
            .unwrap()
            .push(UpdateIssueTitleParams {
                owner: owner.to_string(),
                repo: repo.to_string(),
                issue_number,
                title: title.to_string(),
            });
        Ok(())
    }

    async fn add_labels(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        labels: &[String],
    ) -> Result<()> {
        self.added_labels.lock().unwrap().push(AddLabelsParams {
            owner: owner.to_string(),
            repo: repo.to_string(),
            issue_number,
            labels: labels.to_vec(),
        });
        Ok(())
    }

    async fn remove_label(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        label: &str,
    ) -> Result<()> {
        self.removed_labels.lock().unwrap().push(RemoveLabelParams {
            owner: owner.to_string(),
            repo: repo.to_string(),
            issue_number,
            label: label.to_string(),
        });
        Ok(())
    }
}

#[async_trait::async_trait]
impl CommentClient for MockGitHubClient {
    async fn get_comments(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
    ) -> Result<Vec<Comment>> {
        let key = format!("{owner}/{repo}/{issue_number}");
        Ok(self.comments.get(&key).cloned().unwrap_or_default())
    }

    async fn update_comment(
        &self,
        owner: &str,
        repo: &str,
        comment_id: u64,
        body: &str,
    ) -> Result<()> {
        self.updated_comments
            .lock()
            .unwrap()
            .push(UpdateCommentParams {
                owner: owner.to_string(),
                repo: repo.to_string(),
                comment_id,
                body: body.to_string(),
            });
        Ok(())
    }

    async fn create_comment(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        body: &str,
    ) -> Result<Comment> {
        self.created_comments
            .lock()
            .unwrap()
            .push(CreateCommentParams {
                owner: owner.to_string(),
                repo: repo.to_string(),
                issue_number,
                body: body.to_string(),
            });
        // Return a mock comment
        Ok(Comment {
            id: "IC_mock_new".to_string(),
            database_id: 99999,
            author: self.current_user.as_ref().map(|login| Author {
                login: login.clone(),
            }),
            created_at: chrono::Utc::now(),
            body: body.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gh::issue_agent::models::Label;
    use crate::github::pr::PrState;
    use chrono::{TimeZone, Utc};

    fn create_test_issue(number: i64) -> Issue {
        Issue {
            number,
            title: format!("Test Issue {number}"),
            body: Some(format!("Body of issue {number}")),
            state: "OPEN".to_string(),
            labels: vec![Label {
                name: "bug".to_string(),
            }],
            assignees: vec![],
            milestone: None,
            author: Some(Author {
                login: "testuser".to_string(),
            }),
            created_at: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap(),
        }
    }

    fn create_test_comment(id: &str, database_id: i64, author: &str) -> Comment {
        Comment {
            id: id.to_string(),
            database_id,
            author: Some(Author {
                login: author.to_string(),
            }),
            created_at: Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap(),
            body: format!("Comment body {database_id}"),
        }
    }

    #[tokio::test]
    async fn mock_client_returns_configured_private_status() {
        let client = MockGitHubClient::new()
            .with_private("owner", "public-repo", false)
            .with_private("owner", "private-repo", true);

        assert!(
            !client
                .is_repo_private("owner", "public-repo")
                .await
                .unwrap()
        );
        assert!(
            client
                .is_repo_private("owner", "private-repo")
                .await
                .unwrap()
        );
        // Default to private for unknown repos
        assert!(client.is_repo_private("owner", "unknown").await.unwrap());
    }

    #[tokio::test]
    async fn mock_client_tracks_created_prs() {
        let client = MockGitHubClient::new();

        let params = CreatePrParams {
            owner: "owner".to_string(),
            repo: "repo".to_string(),
            title: "Test PR".to_string(),
            body: "Test body".to_string(),
            head: "feature".to_string(),
            base: Some("main".to_string()),
            draft: false,
        };

        let url = client.create_pull_request(params).await.unwrap();
        assert_eq!(url, "https://github.com/owner/repo/pull/1");

        let created = client.created_prs.lock().unwrap();
        assert_eq!(created.len(), 1);
        assert_eq!(created[0].title, "Test PR");
    }

    #[test]
    fn mock_client_tracks_browser_opens() {
        let client = MockGitHubClient::new();
        client.open_in_browser("https://example.com");

        let opened = client.opened_urls.lock().unwrap();
        assert_eq!(opened.len(), 1);
        assert_eq!(opened[0], "https://example.com");
    }

    #[tokio::test]
    async fn mock_client_returns_branch_pr_info() {
        let client = MockGitHubClient::new().with_branch_pr(
            "owner",
            "repo",
            "feature",
            PrInfo {
                state: PrState::Open,
                url: "https://github.com/owner/repo/pull/1".to_string(),
            },
        );

        let pr_info = client
            .get_pr_for_branch("owner", "repo", "feature")
            .await
            .unwrap();
        assert!(pr_info.is_some());
        let pr_info = pr_info.unwrap();
        assert_eq!(pr_info.state, PrState::Open);

        // Unknown branch returns None
        let no_pr = client
            .get_pr_for_branch("owner", "repo", "unknown")
            .await
            .unwrap();
        assert!(no_pr.is_none());
    }

    #[tokio::test]
    async fn mock_client_returns_configured_default_branch() {
        let client = MockGitHubClient::new()
            .with_default_branch("owner", "main-repo", "main")
            .with_default_branch("owner", "master-repo", "master")
            .with_default_branch("owner", "custom-repo", "develop");

        assert_eq!(
            client
                .get_default_branch("owner", "main-repo")
                .await
                .unwrap(),
            "main"
        );
        assert_eq!(
            client
                .get_default_branch("owner", "master-repo")
                .await
                .unwrap(),
            "master"
        );
        assert_eq!(
            client
                .get_default_branch("owner", "custom-repo")
                .await
                .unwrap(),
            "develop"
        );
        // Default to "main" for unknown repos
        assert_eq!(
            client.get_default_branch("owner", "unknown").await.unwrap(),
            "main"
        );
    }

    #[tokio::test]
    async fn mock_client_returns_configured_issue() {
        let issue = create_test_issue(123);
        let client = MockGitHubClient::new().with_issue("owner", "repo", issue.clone());

        let fetched = client.get_issue("owner", "repo", 123).await.unwrap();
        assert_eq!(fetched.number, 123);
        assert_eq!(fetched.title, "Test Issue 123");
    }

    #[tokio::test]
    async fn mock_client_returns_error_for_unknown_issue() {
        let client = MockGitHubClient::new();
        let result = client.get_issue("owner", "repo", 999).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn mock_client_returns_configured_comments() {
        let comments = vec![
            create_test_comment("IC_1", 1001, "user1"),
            create_test_comment("IC_2", 1002, "user2"),
        ];
        let client = MockGitHubClient::new().with_comments("owner", "repo", 123, comments);

        let fetched = client.get_comments("owner", "repo", 123).await.unwrap();
        assert_eq!(fetched.len(), 2);
        assert_eq!(fetched[0].database_id, 1001);
        assert_eq!(fetched[1].database_id, 1002);
    }

    #[tokio::test]
    async fn mock_client_returns_empty_comments_for_unknown_issue() {
        let client = MockGitHubClient::new();
        let comments = client.get_comments("owner", "repo", 999).await.unwrap();
        assert!(comments.is_empty());
    }

    #[tokio::test]
    async fn mock_client_tracks_update_issue_body() {
        let client = MockGitHubClient::new();
        client
            .update_issue_body("owner", "repo", 123, "New body")
            .await
            .unwrap();

        let updated = client.updated_issue_bodies.lock().unwrap();
        assert_eq!(updated.len(), 1);
        assert_eq!(updated[0].owner, "owner");
        assert_eq!(updated[0].repo, "repo");
        assert_eq!(updated[0].issue_number, 123);
        assert_eq!(updated[0].body, "New body");
    }

    #[tokio::test]
    async fn mock_client_tracks_update_issue_title() {
        let client = MockGitHubClient::new();
        client
            .update_issue_title("owner", "repo", 123, "New title")
            .await
            .unwrap();

        let updated = client.updated_issue_titles.lock().unwrap();
        assert_eq!(updated.len(), 1);
        assert_eq!(updated[0].title, "New title");
    }

    #[tokio::test]
    async fn mock_client_tracks_add_labels() {
        let client = MockGitHubClient::new();
        let labels = vec!["bug".to_string(), "urgent".to_string()];
        client
            .add_labels("owner", "repo", 123, &labels)
            .await
            .unwrap();

        let added = client.added_labels.lock().unwrap();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].labels, vec!["bug", "urgent"]);
    }

    #[tokio::test]
    async fn mock_client_tracks_remove_label() {
        let client = MockGitHubClient::new();
        client
            .remove_label("owner", "repo", 123, "bug")
            .await
            .unwrap();

        let removed = client.removed_labels.lock().unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].label, "bug");
    }

    #[tokio::test]
    async fn mock_client_tracks_update_comment() {
        let client = MockGitHubClient::new();
        client
            .update_comment("owner", "repo", 456, "Updated body")
            .await
            .unwrap();

        let updated = client.updated_comments.lock().unwrap();
        assert_eq!(updated.len(), 1);
        assert_eq!(updated[0].comment_id, 456);
        assert_eq!(updated[0].body, "Updated body");
    }

    #[tokio::test]
    async fn mock_client_tracks_create_comment() {
        let client = MockGitHubClient::new().with_current_user("testuser");
        let comment = client
            .create_comment("owner", "repo", 123, "New comment")
            .await
            .unwrap();

        let created = client.created_comments.lock().unwrap();
        assert_eq!(created.len(), 1);
        assert_eq!(created[0].body, "New comment");

        assert_eq!(comment.body, "New comment");
        assert_eq!(comment.author.unwrap().login, "testuser");
    }
}
