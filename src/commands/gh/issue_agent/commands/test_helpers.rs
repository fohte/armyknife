//! Test helpers shared across issue-agent command tests.

use crate::commands::gh::issue_agent::models::{Comment, Issue, IssueMetadata};
use crate::commands::gh::issue_agent::storage::IssueStorage;
use crate::commands::gh::issue_agent::testing::factories;
use crate::infra::github::MockGitHubClient;
use chrono::{TimeZone, Utc};
use rstest::fixture;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Create a temporary directory for test storage.
#[fixture]
pub fn test_dir() -> TempDir {
    tempfile::tempdir().unwrap()
}

/// Create a standard test issue with common defaults.
/// Uses fixed timestamps for deterministic tests.
#[fixture]
pub fn test_issue() -> Issue {
    factories::issue_with(|i| {
        i.number = 123;
        i.title = "Test Issue".to_string();
        i.body = Some("Test body content".to_string());
        i.labels = factories::labels(&["bug"]);
        i.assignees = factories::assignees(&["assignee1"]);
        i.created_at = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        i.updated_at = Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap();
    })
}

/// Create a standard test comment.
/// Uses fixed timestamps for deterministic tests.
#[fixture]
pub fn test_comment() -> Comment {
    factories::comment_with(|c| {
        c.id = "IC_abc123".to_string();
        c.database_id = 12345;
        c.author = Some(factories::author("commenter"));
        c.created_at = Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap();
        c.body = "Test comment body".to_string();
    })
}

/// Create a test issue with configurable fields.
pub fn create_test_issue(number: i64, title: &str, body: &str, updated_at: &str) -> Issue {
    factories::issue_with(|i| {
        i.number = number;
        i.title = title.to_string();
        i.body = Some(body.to_string());
        i.labels = factories::labels(&["bug"]);
        i.created_at = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        i.updated_at = chrono::DateTime::parse_from_rfc3339(updated_at)
            .unwrap()
            .with_timezone(&Utc);
    })
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

/// Create a comment with the given author (other fields use defaults).
pub fn make_comment(author: &str) -> Comment {
    factories::comment_with(|c| {
        c.id = "IC_abc".to_string();
        c.database_id = 12345;
        c.author = Some(factories::author(author));
        c.created_at = Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap();
        c.body = "Original".to_string();
    })
}

/// Write a local comment file to the storage directory.
pub fn setup_local_comment(dir: &Path, filename: &str, content: &str) {
    let comments_dir = dir.join("comments");
    fs::create_dir_all(&comments_dir).unwrap();
    fs::write(comments_dir.join(filename), content).unwrap();
}

// Default timestamps for tests
pub const DEFAULT_TS: &str = "2024-01-02T00:00:00+00:00";
pub const OLD_TS: &str = "2024-01-01T00:00:00+00:00";

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
/// Use this to set up MockGitHubClient and local storage for integration tests.
/// Only specify the fields you want to change from defaults.
///
/// # Example
/// ```ignore
/// let (client, storage) = TestSetup::new(test_dir.path())
///     .remote_title("Old Title")
///     .local_title("New Title")
///     .build();
/// ```
#[allow(dead_code)] // All setters are generated but not all may be used yet
pub struct TestSetup<'a> {
    dir: &'a Path,
    // Remote state (what GitHub API returns)
    pub remote_title: &'a str,
    pub remote_body: &'a str,
    pub remote_ts: &'a str,
    pub remote_comments: Vec<Comment>,
    // Local state (what's in the storage directory)
    pub local_title: &'a str,
    pub local_body: &'a str,
    pub local_labels: Vec<&'a str>,
    pub local_ts: &'a str,
    // Mock client config
    pub current_user: &'a str,
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
        remote_comments: Vec<Comment>,
        local_title: &'a str,
        local_body: &'a str,
        local_labels: Vec<&'a str>,
        local_ts: &'a str,
        current_user: &'a str,
    }

    /// Build the MockGitHubClient and IssueStorage.
    pub fn build(self) -> (MockGitHubClient, IssueStorage) {
        let issue = create_test_issue(123, self.remote_title, self.remote_body, self.remote_ts);
        let client = MockGitHubClient::new()
            .with_issue("owner", "repo", issue)
            .with_comments("owner", "repo", 123, self.remote_comments)
            .with_current_user(self.current_user);

        fs::create_dir_all(self.dir).unwrap();
        fs::write(self.dir.join("issue.md"), format!("{}\n", self.local_body)).unwrap();
        fs::write(
            self.dir.join("metadata.json"),
            create_metadata_json(123, self.local_title, self.local_ts, &self.local_labels),
        )
        .unwrap();

        let storage = IssueStorage::from_dir(self.dir);
        (client, storage)
    }
}
