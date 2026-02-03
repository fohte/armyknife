use std::collections::HashMap;

use super::error::Result;
use super::issue_storage::IssueStorage;
use crate::commands::gh::issue_agent::models::{Comment, Issue};

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

impl IssueStorage {
    /// Detect local changes compared to remote data, returning detailed change info.
    pub fn detect_changes(
        &self,
        remote_issue: &Issue,
        remote_comments: &[Comment],
    ) -> Result<LocalChanges> {
        let mut changes = LocalChanges::default();

        // Check issue body
        if let Ok(local_body) = self.read_body() {
            let remote_body = remote_issue.body.as_deref().unwrap_or("");
            if local_body != remote_body {
                changes.body_changed = true;
            }
        }

        // Check metadata (title)
        if let Ok(local_metadata) = self.read_metadata()
            && local_metadata.title != remote_issue.title
        {
            changes.title_changed = true;
        }

        // Check comments using HashMap for O(1) lookups
        if let Ok(local_comments) = self.read_comments() {
            let remote_comments_map: HashMap<&str, &Comment> =
                remote_comments.iter().map(|c| (c.id.as_str(), c)).collect();

            for local_comment in &local_comments {
                if local_comment.is_new() {
                    // New comment file
                    changes
                        .new_comment_files
                        .push(local_comment.filename.clone());
                } else if let Some(comment_id) = &local_comment.metadata.id
                    && let Some(remote_comment) = remote_comments_map.get(comment_id.as_str())
                    // Compare with whitespace normalized to handle inconsistencies
                    // between GitHub API responses and local file parsing
                    && local_comment.body.trim() != remote_comment.body.trim()
                {
                    // Local comment differs from remote
                    changes.modified_comment_ids.push(comment_id.clone());
                }
            }
        }

        Ok(changes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::gh::issue_agent::models::{Author, Label};
    use chrono::{TimeZone, Utc};
    use indoc::indoc;
    use rstest::{fixture, rstest};
    use std::fs;
    use tempfile::TempDir;

    #[fixture]
    fn test_dir() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    #[fixture]
    fn test_issue() -> Issue {
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
            body_last_edited_at: None,
            title_last_edited_at: None,
        }
    }

    #[fixture]
    fn test_comment() -> Comment {
        Comment {
            id: "IC_abc123".to_string(),
            database_id: 12345,
            author: Some(Author {
                login: "testuser".to_string(),
            }),
            created_at: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            body: "Original comment".to_string(),
        }
    }

    #[rstest]
    fn test_no_changes(test_dir: TempDir, test_issue: Issue, test_comment: Comment) {
        let storage = IssueStorage::from_dir(test_dir.path());

        // Write matching local files
        fs::write(test_dir.path().join("issue.md"), "Original body\n").unwrap();
        fs::write(
            test_dir.path().join("metadata.json"),
            indoc! {r#"
                {"number":123,"title":"Test Issue","state":"OPEN","labels":[],"assignees":[],"milestone":null,"author":"testuser","createdAt":"2024-01-01T00:00:00Z","updatedAt":"2024-01-02T00:00:00Z"}
            "#}.trim(),
        )
        .unwrap();

        let comments_dir = test_dir.path().join("comments");
        fs::create_dir(&comments_dir).unwrap();
        fs::write(
            comments_dir.join("001_comment_12345.md"),
            indoc! {"
                <!-- author: testuser -->
                <!-- createdAt: 2024-01-01T00:00:00Z -->
                <!-- id: IC_abc123 -->
                <!-- databaseId: 12345 -->

                Original comment
            "},
        )
        .unwrap();

        let changes = storage
            .detect_changes(&test_issue, &[test_comment])
            .unwrap();
        assert!(!changes.has_changes());
    }

    #[rstest]
    fn test_body_changed(test_dir: TempDir, test_issue: Issue) {
        let storage = IssueStorage::from_dir(test_dir.path());
        fs::write(test_dir.path().join("issue.md"), "Modified body\n").unwrap();

        let changes = storage.detect_changes(&test_issue, &[]).unwrap();
        assert!(changes.has_changes());
        assert!(changes.body_changed);
    }

    #[rstest]
    fn test_title_changed(test_dir: TempDir, test_issue: Issue) {
        let storage = IssueStorage::from_dir(test_dir.path());
        fs::write(
            test_dir.path().join("metadata.json"),
            indoc! {r#"
                {"number":123,"title":"Modified Title","state":"OPEN","labels":[],"assignees":[],"milestone":null,"author":"testuser","createdAt":"2024-01-01T00:00:00Z","updatedAt":"2024-01-02T00:00:00Z"}
            "#}.trim(),
        )
        .unwrap();

        let changes = storage.detect_changes(&test_issue, &[]).unwrap();
        assert!(changes.has_changes());
        assert!(changes.title_changed);
    }

    #[rstest]
    fn test_comment_modified(test_dir: TempDir, test_issue: Issue, test_comment: Comment) {
        let storage = IssueStorage::from_dir(test_dir.path());
        let comments_dir = test_dir.path().join("comments");
        fs::create_dir(&comments_dir).unwrap();
        fs::write(
            comments_dir.join("001_comment_12345.md"),
            indoc! {"
                <!-- author: testuser -->
                <!-- createdAt: 2024-01-01T00:00:00Z -->
                <!-- id: IC_abc123 -->
                <!-- databaseId: 12345 -->

                Modified comment
            "},
        )
        .unwrap();

        let changes = storage
            .detect_changes(&test_issue, &[test_comment])
            .unwrap();
        assert!(changes.has_changes());
        assert_eq!(changes.modified_comment_ids, vec!["IC_abc123"]);
    }

    #[rstest]
    fn test_new_comment(test_dir: TempDir, test_issue: Issue) {
        let storage = IssueStorage::from_dir(test_dir.path());
        let comments_dir = test_dir.path().join("comments");
        fs::create_dir(&comments_dir).unwrap();
        fs::write(
            comments_dir.join("new_my_comment.md"),
            "New comment content",
        )
        .unwrap();

        let changes = storage.detect_changes(&test_issue, &[]).unwrap();
        assert!(changes.has_changes());
        assert_eq!(changes.new_comment_files, vec!["new_my_comment.md"]);
    }

    /// Test that comments with whitespace-only differences are not detected as changed.
    /// GitHub API may return body with leading/trailing newlines, but local file parsing
    /// normalizes them via lines().join("\n").
    #[rstest]
    #[case::trailing_newline("Original comment", "Original comment\n")]
    #[case::trailing_multiple_newlines("Original comment", "Original comment\n\n")]
    #[case::trailing_spaces("Original comment", "Original comment  ")]
    #[case::leading_newline("Original comment", "\nOriginal comment")]
    #[case::leading_multiple_newlines("Original comment", "\n\nOriginal comment")]
    fn test_no_change_with_whitespace_difference(
        test_dir: TempDir,
        test_issue: Issue,
        #[case] local_body: &str,
        #[case] remote_body: &str,
    ) {
        let storage = IssueStorage::from_dir(test_dir.path());
        let comments_dir = test_dir.path().join("comments");
        fs::create_dir(&comments_dir).unwrap();
        fs::write(
            comments_dir.join("001_comment_12345.md"),
            format!(
                indoc! {"
                    <!-- author: testuser -->
                    <!-- createdAt: 2024-01-01T00:00:00Z -->
                    <!-- id: IC_abc123 -->
                    <!-- databaseId: 12345 -->

                    {}
                "},
                local_body
            ),
        )
        .unwrap();

        let comment = Comment {
            id: "IC_abc123".to_string(),
            database_id: 12345,
            author: Some(Author {
                login: "testuser".to_string(),
            }),
            created_at: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            body: remote_body.to_string(),
        };

        let changes = storage.detect_changes(&test_issue, &[comment]).unwrap();
        assert!(
            !changes.has_changes(),
            "Expected no changes, but got: {:?}",
            changes
        );
    }
}
