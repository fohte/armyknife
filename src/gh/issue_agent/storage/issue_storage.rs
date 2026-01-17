use std::fs;
use std::path::{Path, PathBuf};

use super::error::{Result, StorageError};
use super::paths::get_issue_dir;
use super::read::{CommentFileMetadata, LocalComment};
use crate::gh::issue_agent::models::{Comment, Issue, IssueMetadata};

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

/// Storage handler for a single GitHub issue.
///
/// Provides read/write operations for issue data stored locally.
/// Directory structure:
/// ```text
/// <cache_dir>/<owner>/<repo>/<issue_number>/
/// ├── issue.md
/// ├── metadata.json
/// └── comments/
///     ├── 001_comment_<databaseId>.md
///     └── new_<name>.md
/// ```
#[derive(Debug, Clone)]
pub struct IssueStorage {
    dir: PathBuf,
}

impl IssueStorage {
    /// Create a new IssueStorage for the given repo and issue number.
    /// Uses the default cache directory.
    pub fn new(repo: &str, issue_number: i64) -> Self {
        Self {
            dir: get_issue_dir(repo, issue_number),
        }
    }

    /// Create an IssueStorage from an existing directory path.
    /// Useful for testing or custom storage locations.
    pub fn from_dir(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Returns the directory path for this issue storage.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    // =========================================================================
    // Read operations
    // =========================================================================

    /// Read the issue body from issue.md.
    pub fn read_body(&self) -> Result<String> {
        let path = self.dir.join("issue.md");
        if !path.exists() {
            return Err(StorageError::FileNotFound(path));
        }
        let content = fs::read_to_string(&path)?;
        // Trim trailing newline added during save
        Ok(content.trim_end_matches('\n').to_string())
    }

    /// Read metadata from metadata.json.
    pub fn read_metadata(&self) -> Result<IssueMetadata> {
        let path = self.dir.join("metadata.json");
        if !path.exists() {
            return Err(StorageError::FileNotFound(path));
        }
        let content = fs::read_to_string(&path)?;
        let metadata: IssueMetadata = serde_json::from_str(&content)?;
        Ok(metadata)
    }

    /// Read all comments from the comments/ directory.
    pub fn read_comments(&self) -> Result<Vec<LocalComment>> {
        let comments_dir = self.dir.join("comments");
        if !comments_dir.exists() {
            return Ok(Vec::new());
        }

        let mut comments = Vec::new();
        let entries = fs::read_dir(&comments_dir)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if path.extension().is_some_and(|ext| ext == "md") {
                let filename = match path.file_name().and_then(|n| n.to_str()) {
                    Some(s) => s.to_string(),
                    None => continue, // Skip files with non-UTF8 names
                };

                let content = fs::read_to_string(&path)?;
                comments.push(LocalComment::parse(&content, filename, &path)?);
            }
        }

        // Sort by filename for consistent ordering
        comments.sort_by(|a, b| a.filename.cmp(&b.filename));

        Ok(comments)
    }

    // =========================================================================
    // Write operations
    // =========================================================================

    /// Save the issue body to issue.md.
    pub fn save_body(&self, body: &str) -> Result<()> {
        fs::create_dir_all(&self.dir)?;
        let path = self.dir.join("issue.md");
        fs::write(&path, format!("{}\n", body))?;
        Ok(())
    }

    /// Save metadata to metadata.json.
    pub fn save_metadata(&self, metadata: &IssueMetadata) -> Result<()> {
        fs::create_dir_all(&self.dir)?;
        let path = self.dir.join("metadata.json");
        let json = serde_json::to_string_pretty(metadata)?;
        fs::write(&path, json)?;
        Ok(())
    }

    /// Save comments to the comments/ directory.
    pub fn save_comments(&self, comments: &[Comment]) -> Result<()> {
        let comments_dir = self.dir.join("comments");
        fs::create_dir_all(&comments_dir)?;

        for (i, comment) in comments.iter().enumerate() {
            let index = format!("{:03}", i + 1);
            let filename = format!("{}_comment_{}.md", index, comment.database_id);
            let path = comments_dir.join(&filename);

            let content = comment.to_file_content();
            fs::write(&path, content)?;
        }

        Ok(())
    }

    // =========================================================================
    // Diff operations
    // =========================================================================

    /// Check if there are local changes compared to remote data.
    pub fn has_changes(&self, remote_issue: &Issue, remote_comments: &[Comment]) -> Result<bool> {
        let changes = self.detect_changes(remote_issue, remote_comments)?;
        Ok(changes.has_changes())
    }

    /// Detect local changes compared to remote data, returning detailed change info.
    pub fn detect_changes(
        &self,
        remote_issue: &Issue,
        remote_comments: &[Comment],
    ) -> Result<LocalChanges> {
        use std::collections::HashMap;

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
                    && local_comment.body != remote_comment.body
                {
                    // Local comment differs from remote
                    changes.modified_comment_ids.push(comment_id.clone());
                }
            }
        }

        Ok(changes)
    }
}

/// Extension trait for Comment to support file serialization.
trait CommentExt {
    fn to_file_content(&self) -> String;
}

impl CommentExt for Comment {
    fn to_file_content(&self) -> String {
        let author = self
            .author
            .as_ref()
            .map(|a| a.login.as_str())
            .unwrap_or("unknown");
        let created_at = self.created_at.to_rfc3339();

        format!(
            "<!-- author: {} -->\n<!-- createdAt: {} -->\n<!-- id: {} -->\n<!-- databaseId: {} -->\n\n{}",
            author, created_at, self.id, self.database_id, self.body
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gh::issue_agent::models::{Author, Label};
    use chrono::{TimeZone, Utc};
    use rstest::{fixture, rstest};
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
            body: "Original comment".to_string(),
        }
    }

    #[fixture]
    fn test_metadata() -> IssueMetadata {
        IssueMetadata {
            number: 123,
            title: "Test Issue".to_string(),
            state: "OPEN".to_string(),
            labels: vec!["bug".to_string()],
            assignees: vec!["user1".to_string()],
            milestone: None,
            author: "author1".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-02T00:00:00Z".to_string(),
        }
    }

    // =========================================================================
    // Read tests
    // =========================================================================

    #[rstest]
    fn test_read_body(test_dir: TempDir) {
        let storage = IssueStorage::from_dir(test_dir.path());
        fs::write(test_dir.path().join("issue.md"), "Test issue body\n").unwrap();

        let body = storage.read_body().unwrap();
        assert_eq!(body, "Test issue body");
    }

    #[rstest]
    fn test_read_metadata(test_dir: TempDir) {
        let storage = IssueStorage::from_dir(test_dir.path());
        let metadata_json = r#"{
            "number": 123,
            "title": "Test Issue",
            "state": "OPEN",
            "labels": ["bug"],
            "assignees": ["user1"],
            "milestone": null,
            "author": "author1",
            "createdAt": "2024-01-01T00:00:00Z",
            "updatedAt": "2024-01-02T00:00:00Z"
        }"#;
        fs::write(test_dir.path().join("metadata.json"), metadata_json).unwrap();

        let metadata = storage.read_metadata().unwrap();
        assert_eq!(metadata.number, 123);
        assert_eq!(metadata.title, "Test Issue");
        assert_eq!(metadata.state, "OPEN");
    }

    #[rstest]
    fn test_read_comments(test_dir: TempDir) {
        let storage = IssueStorage::from_dir(test_dir.path());
        let comments_dir = test_dir.path().join("comments");
        fs::create_dir(&comments_dir).unwrap();

        let comment_content = r#"<!-- author: testuser -->
<!-- createdAt: 2024-01-01T00:00:00Z -->
<!-- id: IC_abc123 -->
<!-- databaseId: 12345 -->

This is the comment body."#;

        fs::write(comments_dir.join("001_comment_12345.md"), comment_content).unwrap();

        let comments = storage.read_comments().unwrap();
        assert_eq!(comments.len(), 1);

        let comment = &comments[0];
        assert_eq!(comment.filename, "001_comment_12345.md");
        assert_eq!(comment.metadata.author, Some("testuser".to_string()));
        assert_eq!(comment.metadata.database_id, Some(12345));
        assert_eq!(comment.body, "This is the comment body.");
        assert!(!comment.is_new());
    }

    #[rstest]
    fn test_read_new_comment(test_dir: TempDir) {
        let storage = IssueStorage::from_dir(test_dir.path());
        let comments_dir = test_dir.path().join("comments");
        fs::create_dir(&comments_dir).unwrap();

        fs::write(
            comments_dir.join("new_my_comment.md"),
            "New comment content",
        )
        .unwrap();

        let comments = storage.read_comments().unwrap();
        assert_eq!(comments.len(), 1);
        assert!(comments[0].is_new());
        assert_eq!(comments[0].body, "New comment content");
    }

    // =========================================================================
    // Write tests
    // =========================================================================

    #[rstest]
    fn test_save_body(test_dir: TempDir) {
        let storage = IssueStorage::from_dir(test_dir.path());
        storage.save_body("Test body content").unwrap();

        let content = fs::read_to_string(test_dir.path().join("issue.md")).unwrap();
        assert_eq!(content, "Test body content\n");
    }

    #[rstest]
    fn test_save_metadata(test_dir: TempDir, test_metadata: IssueMetadata) {
        let storage = IssueStorage::from_dir(test_dir.path());
        storage.save_metadata(&test_metadata).unwrap();

        let content = fs::read_to_string(test_dir.path().join("metadata.json")).unwrap();
        let loaded: IssueMetadata = serde_json::from_str(&content).unwrap();
        assert_eq!(loaded.number, 123);
        assert_eq!(loaded.title, "Test Issue");
    }

    #[rstest]
    fn test_save_comments(test_dir: TempDir) {
        let storage = IssueStorage::from_dir(test_dir.path());
        let comments = vec![
            Comment {
                id: "IC_abc123".to_string(),
                database_id: 12345,
                author: Some(Author {
                    login: "testuser".to_string(),
                }),
                created_at: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
                body: "First comment".to_string(),
            },
            Comment {
                id: "IC_def456".to_string(),
                database_id: 67890,
                author: None,
                created_at: Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap(),
                body: "Second comment".to_string(),
            },
        ];

        storage.save_comments(&comments).unwrap();

        let comments_dir = test_dir.path().join("comments");
        assert!(comments_dir.exists());

        let first_comment = fs::read_to_string(comments_dir.join("001_comment_12345.md")).unwrap();
        assert!(first_comment.contains("<!-- author: testuser -->"));
        assert!(first_comment.contains("<!-- databaseId: 12345 -->"));
        assert!(first_comment.contains("First comment"));

        let second_comment = fs::read_to_string(comments_dir.join("002_comment_67890.md")).unwrap();
        assert!(second_comment.contains("<!-- author: unknown -->"));
        assert!(second_comment.contains("Second comment"));
    }

    // =========================================================================
    // Diff tests
    // =========================================================================

    #[rstest]
    fn test_no_changes(test_dir: TempDir, test_issue: Issue, test_comment: Comment) {
        let storage = IssueStorage::from_dir(test_dir.path());

        // Write matching local files
        fs::write(test_dir.path().join("issue.md"), "Original body\n").unwrap();
        fs::write(
            test_dir.path().join("metadata.json"),
            r#"{"number":123,"title":"Test Issue","state":"OPEN","labels":[],"assignees":[],"milestone":null,"author":"testuser","createdAt":"2024-01-01T00:00:00Z","updatedAt":"2024-01-02T00:00:00Z"}"#,
        )
        .unwrap();

        let comments_dir = test_dir.path().join("comments");
        fs::create_dir(&comments_dir).unwrap();
        fs::write(
            comments_dir.join("001_comment_12345.md"),
            "<!-- author: testuser -->\n<!-- createdAt: 2024-01-01T00:00:00Z -->\n<!-- id: IC_abc123 -->\n<!-- databaseId: 12345 -->\n\nOriginal comment",
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
            r#"{"number":123,"title":"Modified Title","state":"OPEN","labels":[],"assignees":[],"milestone":null,"author":"testuser","createdAt":"2024-01-01T00:00:00Z","updatedAt":"2024-01-02T00:00:00Z"}"#,
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
            "<!-- author: testuser -->\n<!-- createdAt: 2024-01-01T00:00:00Z -->\n<!-- id: IC_abc123 -->\n<!-- databaseId: 12345 -->\n\nModified comment",
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
}
