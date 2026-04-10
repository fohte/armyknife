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

/// Normalize body text for comparison.
///
/// GitHub API may return body with trailing newlines or CRLF line endings that
/// are not preserved in local files after pull (or vice versa). Normalizing
/// before comparison prevents false positives where a no-op pull would be
/// detected as a body change.
///
/// Leading whitespace is intentionally preserved because it can be
/// semantically significant in markdown (e.g., indented code blocks).
fn normalize_body_for_compare(body: &str) -> String {
    body.replace("\r\n", "\n")
        .trim_end_matches(['\n', '\r', ' ', '\t'])
        .to_string()
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
            if normalize_body_for_compare(&local_body) != normalize_body_for_compare(remote_body) {
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
            last_edited_at: None,
            parent_issue: None,
            sub_issues: vec![],
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

    /// Pull writes the raw remote body into issue.md, and `read_body` strips
    /// trailing newlines on read. Without normalization in detect_changes, a
    /// no-op pull whose remote body ends with `\n` or `\n\n` would always be
    /// reported as a body change.
    // Use `concat!` to split multi-escape string literals so that the
    // `prefer-indoc` lint (which targets literals with 2+ `\n` escapes)
    // does not fire on intentionally control-character-heavy test data.
    #[rstest]
    #[case::remote_trailing_newline("Original body", "Original body\n")]
    #[case::remote_trailing_two_newlines("Original body", concat!("Original body\n", "\n"))]
    #[case::remote_crlf("Original body", "Original body\r\n")]
    #[case::remote_trailing_spaces("Original body", "Original body  ")]
    #[case::remote_multiline_crlf("line 1\nline 2", concat!("line 1\r\n", "line 2\r\n"))]
    fn test_body_unchanged_with_whitespace_difference(
        test_dir: TempDir,
        #[case] local_body: &str,
        #[case] remote_body: &str,
    ) {
        let storage = IssueStorage::from_dir(test_dir.path());
        // Use the legacy body-only format. read_body strips trailing newlines
        // so the local side mirrors what we would see after a clean pull.
        fs::write(test_dir.path().join("issue.md"), local_body).unwrap();

        let issue = Issue {
            number: 123,
            title: "Test Issue".to_string(),
            body: Some(remote_body.to_string()),
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
            last_edited_at: None,
            parent_issue: None,
            sub_issues: vec![],
        };

        let changes = storage.detect_changes(&issue, &[]).unwrap();
        assert!(
            !changes.body_changed,
            "Expected body_changed=false for local={:?}, remote={:?}",
            local_body, remote_body,
        );
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
    #[case::trailing_multiple_newlines("Original comment", indoc! {"
        Original comment

    "})]
    #[case::trailing_spaces("Original comment", "Original comment  ")]
    #[case::leading_newline("Original comment", "\nOriginal comment")]
    #[case::leading_multiple_newlines("Original comment", indoc! {"


        Original comment"})]
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
