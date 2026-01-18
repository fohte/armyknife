//! Test helpers shared across issue-agent command tests.

use crate::gh::issue_agent::models::{Author, Comment, Issue, IssueMetadata, Label};
use chrono::{TimeZone, Utc};
use rstest::fixture;
use tempfile::TempDir;

/// Create a temporary directory for test storage.
#[fixture]
pub fn test_dir() -> TempDir {
    tempfile::tempdir().unwrap()
}

/// Create a standard test issue with common defaults.
#[fixture]
pub fn test_issue() -> Issue {
    Issue {
        number: 123,
        title: "Test Issue".to_string(),
        body: Some("Test body content".to_string()),
        state: "OPEN".to_string(),
        labels: vec![Label {
            name: "bug".to_string(),
        }],
        assignees: vec![Author {
            login: "assignee1".to_string(),
        }],
        milestone: None,
        author: Some(Author {
            login: "testuser".to_string(),
        }),
        created_at: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        updated_at: Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap(),
    }
}

/// Create a standard test comment.
#[fixture]
pub fn test_comment() -> Comment {
    Comment {
        id: "IC_abc123".to_string(),
        database_id: 12345,
        author: Some(Author {
            login: "commenter".to_string(),
        }),
        created_at: Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap(),
        body: "Test comment body".to_string(),
    }
}

/// Create a test issue with configurable fields.
///
/// This provides more flexibility than the `test_issue` fixture when
/// tests need specific values for number, title, body, or updated_at.
pub fn create_test_issue(number: i64, title: &str, body: &str, updated_at: &str) -> Issue {
    Issue {
        number,
        title: title.to_string(),
        body: Some(body.to_string()),
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
        updated_at: chrono::DateTime::parse_from_rfc3339(updated_at)
            .unwrap()
            .with_timezone(&Utc),
    }
}

/// Create metadata JSON using IssueMetadata serialization.
///
/// This ensures metadata format consistency with the actual implementation
/// and automatically handles field additions.
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
