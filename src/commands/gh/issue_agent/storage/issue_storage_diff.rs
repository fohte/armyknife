use std::collections::{HashMap, HashSet};

use super::error::Result;
use super::issue_storage::IssueStorage;
use crate::commands::gh::issue_agent::body_compare::{bodies_equal, comment_bodies_equal};
use crate::commands::gh::issue_agent::field_comparison::compare_issue_fields;
use crate::commands::gh::issue_agent::models::{Comment, Issue};

/// Local edits that would be overwritten by pulling remote data.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalChanges {
    /// Whether the issue body has been modified.
    pub body_changed: bool,
    /// Whether the title has been modified.
    pub title_changed: bool,
    /// Whether the issue labels have been modified.
    pub labels_changed: bool,
    /// Whether the issue's sub-issues have been modified.
    pub sub_issues_changed: bool,
    /// Whether the issue's parent has been modified.
    pub parent_issue_changed: bool,
    /// IDs of comments that have been modified.
    pub modified_comment_ids: Vec<String>,
    /// IDs of comments removed locally.
    pub deleted_comment_ids: Vec<String>,
    /// Filenames of new comments (new_*.md).
    pub new_comment_files: Vec<String>,
}

impl LocalChanges {
    /// Returns true if pulling would overwrite a local edit.
    pub fn has_changes(&self) -> bool {
        self.body_changed
            || self.title_changed
            || self.labels_changed
            || self.sub_issues_changed
            || self.parent_issue_changed
            || !self.modified_comment_ids.is_empty()
            || !self.deleted_comment_ids.is_empty()
    }
}

impl IssueStorage {
    /// Detect local changes compared to pull-time snapshots, returning detailed change info.
    pub fn detect_changes(
        &self,
        remote_issue: &Issue,
        remote_comments: &[Comment],
    ) -> Result<LocalChanges> {
        let mut changes = LocalChanges::default();
        self.detect_issue_changes(remote_issue, &mut changes);
        self.detect_comment_changes(remote_comments, &mut changes);

        Ok(changes)
    }

    fn detect_issue_changes(&self, remote_issue: &Issue, changes: &mut LocalChanges) {
        if let Some(base_issue) = self.read_base_issue().ok().flatten() {
            let local_body = self.read_body().ok();
            if let Ok(local_metadata) = self.read_metadata() {
                let comparison_body = local_body.as_deref().unwrap_or(&base_issue.body);
                let comparisons = compare_issue_fields(
                    &local_metadata,
                    comparison_body,
                    remote_issue,
                    &base_issue,
                );
                changes.body_changed = comparisons.body.local_changed;
                changes.title_changed = comparisons.title.local_changed;
                changes.labels_changed = comparisons.labels.local_changed;
                changes.sub_issues_changed = comparisons.sub_issues.local_changed;
                changes.parent_issue_changed = comparisons.parent_issue.local_changed;
            } else if let Some(local_body) = local_body {
                changes.body_changed = !bodies_equal(&local_body, &base_issue.body);
            }
        } else {
            if let Ok(local_body) = self.read_body() {
                let remote_body = remote_issue.body.as_deref().unwrap_or("");
                changes.body_changed = !bodies_equal(&local_body, remote_body);
            }

            if let Ok(local_metadata) = self.read_metadata() {
                changes.title_changed = local_metadata.title != remote_issue.title;
            }
        }
    }

    fn detect_comment_changes(&self, remote_comments: &[Comment], changes: &mut LocalChanges) {
        let Ok(local_comments) = self.read_comments() else {
            return;
        };

        changes.new_comment_files.extend(
            local_comments
                .iter()
                .filter(|comment| comment.is_new())
                .map(|comment| comment.filename.clone()),
        );

        let remote_comments_map: HashMap<&str, &Comment> =
            remote_comments.iter().map(|c| (c.id.as_str(), c)).collect();
        if let Some(base_comments) = self.read_base_comments().ok().flatten() {
            let base_comments_map: HashMap<&str, _> = base_comments
                .iter()
                .map(|comment| (comment.id.as_str(), comment))
                .collect();
            let local_comment_ids: HashSet<&str> = local_comments
                .iter()
                .filter(|comment| !comment.is_new())
                .filter_map(|comment| comment.metadata.id.as_deref())
                .collect();

            for local_comment in local_comments.iter().filter(|comment| !comment.is_new()) {
                if let Some(comment_id) = &local_comment.metadata.id {
                    let base_comment = base_comments_map.get(comment_id.as_str());
                    let remote_comment = remote_comments_map.get(comment_id.as_str());
                    let changed = match (base_comment, remote_comment) {
                        (Some(base), _) => !comment_bodies_equal(&local_comment.body, &base.body),
                        (None, Some(remote)) => {
                            !comment_bodies_equal(&local_comment.body, &remote.body)
                        }
                        (None, None) => true,
                    };
                    if changed {
                        changes.modified_comment_ids.push(comment_id.clone());
                    }
                }
            }

            for base_comment in &base_comments {
                if !local_comment_ids.contains(base_comment.id.as_str()) {
                    changes.deleted_comment_ids.push(base_comment.id.clone());
                }
            }
        } else {
            for local_comment in local_comments.iter().filter(|comment| !comment.is_new()) {
                if let Some(comment_id) = &local_comment.metadata.id
                    && let Some(remote_comment) = remote_comments_map.get(comment_id.as_str())
                    && !comment_bodies_equal(&local_comment.body, &remote_comment.body)
                {
                    changes.modified_comment_ids.push(comment_id.clone());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::gh::issue_agent::models::{Author, IssueFrontmatter, Label, SubIssueRef};
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

    #[rstest]
    fn test_detects_local_issue_field_changes_from_snapshot(test_dir: TempDir, test_issue: Issue) {
        let storage = IssueStorage::from_dir(test_dir.path());
        let base_frontmatter = IssueFrontmatter::from_issue(&test_issue);
        let mut local_frontmatter = base_frontmatter.clone();
        local_frontmatter.fields.title = "Modified title".to_string();
        local_frontmatter.fields.labels = vec!["enhancement".to_string()];
        local_frontmatter.fields.sub_issues = vec!["owner/repo#456".to_string()];
        local_frontmatter.fields.parent_issue = Some("owner/repo#789".to_string());
        storage
            .save_issue(&local_frontmatter, "Modified body")
            .unwrap();
        storage
            .save_base_issue(&base_frontmatter, "Original body")
            .unwrap();

        let changes = storage.detect_changes(&test_issue, &[]).unwrap();

        assert_eq!(
            changes,
            LocalChanges {
                body_changed: true,
                title_changed: true,
                labels_changed: true,
                sub_issues_changed: true,
                parent_issue_changed: true,
                ..LocalChanges::default()
            }
        );
    }

    #[rstest]
    fn test_remote_issue_and_comment_changes_do_not_count_as_local_changes(
        test_dir: TempDir,
        test_issue: Issue,
        test_comment: Comment,
    ) {
        let storage = IssueStorage::from_dir(test_dir.path());
        let base_frontmatter = IssueFrontmatter::from_issue(&test_issue);
        storage
            .save_issue(&base_frontmatter, "Original body")
            .unwrap();
        storage
            .save_comments(std::slice::from_ref(&test_comment))
            .unwrap();

        let mut remote_issue = test_issue;
        remote_issue.body = Some("Remote body".to_string());
        remote_issue.title = "Remote title".to_string();
        remote_issue.labels = vec![Label {
            name: "enhancement".to_string(),
        }];
        remote_issue.sub_issues = vec![SubIssueRef {
            id: 456,
            number: 456,
            owner: "owner".to_string(),
            repo: "repo".to_string(),
        }];
        remote_issue.parent_issue = Some(SubIssueRef {
            id: 789,
            number: 789,
            owner: "owner".to_string(),
            repo: "repo".to_string(),
        });
        let remote_comment = Comment {
            body: "Remote comment".to_string(),
            ..test_comment
        };

        assert_eq!(
            storage
                .detect_changes(&remote_issue, &[remote_comment])
                .unwrap(),
            LocalChanges::default()
        );
    }

    #[rstest]
    fn test_corrupt_snapshots_fall_back_to_remote_comparison(
        test_dir: TempDir,
        test_issue: Issue,
        test_comment: Comment,
    ) {
        let storage = IssueStorage::from_dir(test_dir.path());
        storage
            .save_issue(&IssueFrontmatter::from_issue(&test_issue), "Original body")
            .unwrap();
        storage
            .save_comments(std::slice::from_ref(&test_comment))
            .unwrap();
        fs::write(test_dir.path().join(".issue.base.md"), "invalid snapshot").unwrap();
        fs::write(
            test_dir.path().join(".comments.base.json"),
            "invalid snapshot",
        )
        .unwrap();

        assert_eq!(
            storage
                .detect_changes(&test_issue, &[test_comment])
                .unwrap(),
            LocalChanges::default()
        );
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
        mut test_issue: Issue,
        #[case] local_body: &str,
        #[case] remote_body: &str,
    ) {
        let storage = IssueStorage::from_dir(test_dir.path());
        // Use the legacy body-only format. read_body strips trailing newlines
        // so the local side mirrors what we would see after a clean pull.
        fs::write(test_dir.path().join("issue.md"), local_body).unwrap();

        test_issue.body = Some(remote_body.to_string());

        let changes = storage.detect_changes(&test_issue, &[]).unwrap();
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
        assert_eq!(
            (changes.clone(), changes.has_changes()),
            (
                LocalChanges {
                    new_comment_files: vec!["new_my_comment.md".to_string()],
                    ..LocalChanges::default()
                },
                false,
            )
        );
    }

    #[rstest]
    fn test_detects_local_comment_change_from_snapshot(
        test_dir: TempDir,
        test_issue: Issue,
        test_comment: Comment,
    ) {
        let storage = IssueStorage::from_dir(test_dir.path());
        storage
            .save_comments(std::slice::from_ref(&test_comment))
            .unwrap();
        let comments_dir = test_dir.path().join("comments");
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
        let remote_comment = Comment {
            body: "Modified comment".to_string(),
            ..test_comment
        };

        assert_eq!(
            storage
                .detect_changes(&test_issue, &[remote_comment])
                .unwrap(),
            LocalChanges {
                modified_comment_ids: vec!["IC_abc123".to_string()],
                ..LocalChanges::default()
            }
        );
    }

    #[rstest]
    fn test_detects_local_comment_missing_from_snapshot_and_remote(
        test_dir: TempDir,
        test_issue: Issue,
    ) {
        let storage = IssueStorage::from_dir(test_dir.path());
        storage.save_base_comments(&[]).unwrap();
        let comments_dir = test_dir.path().join("comments");
        fs::create_dir_all(&comments_dir).unwrap();
        fs::write(
            comments_dir.join("001_comment_12345.md"),
            indoc! {"
                <!-- author: testuser -->
                <!-- createdAt: 2024-01-01T00:00:00Z -->
                <!-- id: IC_abc123 -->
                <!-- databaseId: 12345 -->

                Local comment
            "},
        )
        .unwrap();

        assert_eq!(
            storage.detect_changes(&test_issue, &[]).unwrap(),
            LocalChanges {
                modified_comment_ids: vec!["IC_abc123".to_string()],
                ..LocalChanges::default()
            }
        );
    }

    #[rstest]
    fn test_detects_locally_deleted_comment_from_snapshot(
        test_dir: TempDir,
        test_issue: Issue,
        test_comment: Comment,
    ) {
        let storage = IssueStorage::from_dir(test_dir.path());
        storage
            .save_comments(std::slice::from_ref(&test_comment))
            .unwrap();
        fs::remove_file(test_dir.path().join("comments/001_comment_12345.md")).unwrap();

        assert_eq!(
            storage
                .detect_changes(&test_issue, &[test_comment])
                .unwrap(),
            LocalChanges {
                deleted_comment_ids: vec!["IC_abc123".to_string()],
                ..LocalChanges::default()
            }
        );
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
