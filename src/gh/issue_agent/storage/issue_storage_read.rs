use std::fs;

use super::error::{Result, StorageError};
use super::issue_storage::IssueStorage;
use super::read::LocalComment;
use crate::gh::issue_agent::models::IssueMetadata;

impl IssueStorage {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::{fixture, rstest};
    use std::fs;
    use tempfile::TempDir;

    #[fixture]
    fn test_dir() -> TempDir {
        tempfile::tempdir().unwrap()
    }

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
}
