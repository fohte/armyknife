//! Test helpers shared across issue-agent command tests.

use crate::commands::gh::issue_agent::models::IssueMetadata;
use crate::commands::gh::issue_agent::storage::IssueStorage;
// Re-export for tests that import from test_helpers
pub use crate::infra::github::{GitHubMockServer, RemoteComment};
use rstest::fixture;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

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
    }

    /// Build the GitHubMockServer and IssueStorage.
    pub async fn build(self) -> (GitHubMockServer, IssueStorage) {
        let mock = GitHubMockServer::start().await;
        let ctx = mock.repo("owner", "repo");

        // Set up remote issue mock
        ctx.issue(123)
            .title(self.remote_title)
            .body(self.remote_body)
            .updated_at(self.remote_ts)
            .get()
            .await;

        // Set up remote comments mock
        ctx.graphql_comments(&self.remote_comments).await;

        // Set up current user mock
        mock.current_user(self.current_user).await;

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
