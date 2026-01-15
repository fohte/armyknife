use std::path::Path;

use super::error::Result;
use super::paths::get_issue_dir;
use super::read::{read_comments_from_dir, read_issue_body_from_dir, read_metadata_from_dir};
use crate::gh::issue_agent::models::{Comment, Issue};

/// Result of change detection between local and remote data.
#[derive(Debug, Clone, Default)]
pub struct LocalChanges {
    /// Whether the issue body has been modified.
    pub body_changed: bool,
    /// Whether the title has been modified.
    pub title_changed: bool,
    /// IDs of comments that have been modified.
    pub modified_comment_ids: Vec<String>,
    /// Filenames of new comments (new_*.md).
    pub new_comment_files: Vec<String>,
}

impl LocalChanges {
    /// Returns true if any local changes exist.
    pub fn has_changes(&self) -> bool {
        self.body_changed
            || self.title_changed
            || !self.modified_comment_ids.is_empty()
            || !self.new_comment_files.is_empty()
    }
}

/// Check if there are local changes compared to remote data.
pub fn has_local_changes(
    repo: &str,
    issue_number: i64,
    remote_issue: &Issue,
    remote_comments: &[Comment],
) -> Result<bool> {
    let issue_dir = get_issue_dir(repo, issue_number);
    let changes = detect_local_changes_from_dir(&issue_dir, remote_issue, remote_comments)?;
    Ok(changes.has_changes())
}

/// Detect local changes compared to remote data, returning detailed change info.
pub fn detect_local_changes(
    repo: &str,
    issue_number: i64,
    remote_issue: &Issue,
    remote_comments: &[Comment],
) -> Result<LocalChanges> {
    let issue_dir = get_issue_dir(repo, issue_number);
    detect_local_changes_from_dir(&issue_dir, remote_issue, remote_comments)
}

/// Detect local changes from a specific directory.
pub fn detect_local_changes_from_dir(
    issue_dir: &Path,
    remote_issue: &Issue,
    remote_comments: &[Comment],
) -> Result<LocalChanges> {
    let mut changes = LocalChanges::default();

    // Check issue body
    if let Ok(local_body) = read_issue_body_from_dir(issue_dir) {
        let remote_body = remote_issue.body.as_deref().unwrap_or("");
        if local_body != remote_body {
            changes.body_changed = true;
        }
    }

    // Check metadata (title)
    if let Ok(local_metadata) = read_metadata_from_dir(issue_dir)
        && local_metadata.title != remote_issue.title
    {
        changes.title_changed = true;
    }

    // Check comments
    if let Ok(local_comments) = read_comments_from_dir(issue_dir) {
        for local_comment in &local_comments {
            if local_comment.is_new() {
                // New comment file
                changes
                    .new_comment_files
                    .push(local_comment.filename.clone());
            } else if let Some(comment_id) = &local_comment.metadata.id
                && let Some(remote_comment) = remote_comments.iter().find(|c| &c.id == comment_id)
                && local_comment.body != remote_comment.body
            {
                // Local comment differs from remote
                changes.modified_comment_ids.push(comment_id.clone());
            }
        }
    }

    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gh::issue_agent::models::{Author, Label};
    use chrono::{TimeZone, Utc};
    use std::fs;
    use tempfile::TempDir;

    fn create_test_issue() -> Issue {
        Issue {
            number: 123,
            title: "Test Issue".to_string(),
            body: Some("Original body".to_string()),
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

    fn create_test_comment() -> Comment {
        Comment {
            id: "IC_abc123".to_string(),
            database_id: 12345,
            author: Some(Author {
                login: "testuser".to_string(),
            }),
            created_at: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            body: "Original comment".to_string(),
        }
    }

    fn setup_test_dir() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn test_no_changes() {
        let dir = setup_test_dir();
        let issue = create_test_issue();
        let comment = create_test_comment();

        // Write matching local files
        fs::write(dir.path().join("issue.md"), "Original body\n").unwrap();
        fs::write(
            dir.path().join("metadata.json"),
            r#"{"number":123,"title":"Test Issue","state":"OPEN","labels":[],"assignees":[],"milestone":null,"author":"testuser","createdAt":"2024-01-01T00:00:00Z","updatedAt":"2024-01-02T00:00:00Z"}"#,
        )
        .unwrap();

        let comments_dir = dir.path().join("comments");
        fs::create_dir(&comments_dir).unwrap();
        fs::write(
            comments_dir.join("001_comment_12345.md"),
            "<!-- author: testuser -->\n<!-- createdAt: 2024-01-01T00:00:00Z -->\n<!-- id: IC_abc123 -->\n<!-- databaseId: 12345 -->\n\nOriginal comment",
        )
        .unwrap();

        let changes = detect_local_changes_from_dir(dir.path(), &issue, &[comment]).unwrap();
        assert!(!changes.has_changes());
    }

    #[test]
    fn test_body_changed() {
        let dir = setup_test_dir();
        let issue = create_test_issue();

        fs::write(dir.path().join("issue.md"), "Modified body\n").unwrap();

        let changes = detect_local_changes_from_dir(dir.path(), &issue, &[]).unwrap();
        assert!(changes.has_changes());
        assert!(changes.body_changed);
    }

    #[test]
    fn test_title_changed() {
        let dir = setup_test_dir();
        let issue = create_test_issue();

        fs::write(
            dir.path().join("metadata.json"),
            r#"{"number":123,"title":"Modified Title","state":"OPEN","labels":[],"assignees":[],"milestone":null,"author":"testuser","createdAt":"2024-01-01T00:00:00Z","updatedAt":"2024-01-02T00:00:00Z"}"#,
        )
        .unwrap();

        let changes = detect_local_changes_from_dir(dir.path(), &issue, &[]).unwrap();
        assert!(changes.has_changes());
        assert!(changes.title_changed);
    }

    #[test]
    fn test_comment_modified() {
        let dir = setup_test_dir();
        let issue = create_test_issue();
        let comment = create_test_comment();

        let comments_dir = dir.path().join("comments");
        fs::create_dir(&comments_dir).unwrap();
        fs::write(
            comments_dir.join("001_comment_12345.md"),
            "<!-- author: testuser -->\n<!-- createdAt: 2024-01-01T00:00:00Z -->\n<!-- id: IC_abc123 -->\n<!-- databaseId: 12345 -->\n\nModified comment",
        )
        .unwrap();

        let changes = detect_local_changes_from_dir(dir.path(), &issue, &[comment]).unwrap();
        assert!(changes.has_changes());
        assert_eq!(changes.modified_comment_ids, vec!["IC_abc123"]);
    }

    #[test]
    fn test_new_comment() {
        let dir = setup_test_dir();
        let issue = create_test_issue();

        let comments_dir = dir.path().join("comments");
        fs::create_dir(&comments_dir).unwrap();
        fs::write(
            comments_dir.join("new_my_comment.md"),
            "New comment content",
        )
        .unwrap();

        let changes = detect_local_changes_from_dir(dir.path(), &issue, &[]).unwrap();
        assert!(changes.has_changes());
        assert_eq!(changes.new_comment_files, vec!["new_my_comment.md"]);
    }
}
