//! Initialization methods for creating new issue/comment boilerplate files.

use std::fs;
use std::path::PathBuf;

use chrono::Local;

use super::error::{Result, StorageError};
use super::issue_storage::IssueStorage;

/// Default content for a new issue file.
const NEW_ISSUE_TEMPLATE: &str = r#"---
title: ""
labels: []
assignees: []
---

Body
"#;

/// Default content for a new comment file.
const NEW_COMMENT_TEMPLATE: &str = "Comment body\n";

impl IssueStorage {
    /// Initialize a new issue boilerplate file.
    ///
    /// Creates the issue.md file with frontmatter template in the "new" directory.
    /// Returns an error if the file already exists.
    pub fn init_new_issue(&self) -> Result<PathBuf> {
        let issue_path = self.dir.join("issue.md");

        if issue_path.exists() {
            return Err(StorageError::FileAlreadyExists(issue_path));
        }

        fs::create_dir_all(&self.dir)?;
        fs::write(&issue_path, NEW_ISSUE_TEMPLATE)?;

        Ok(issue_path)
    }

    /// Initialize a new comment boilerplate file.
    ///
    /// Creates a new_<name>.md file in the comments directory.
    /// If name is None, uses a timestamp (e.g., new_20240101_120000.md).
    /// Returns the path to the created file.
    /// Returns an error if the file already exists.
    pub fn init_new_comment(&self, name: Option<&str>) -> Result<PathBuf> {
        let comments_dir = self.dir.join("comments");
        let filename = match name {
            Some(n) => format!("new_{}.md", n),
            None => {
                let timestamp = Local::now().format("%Y%m%d_%H%M%S");
                format!("new_{}.md", timestamp)
            }
        };

        let comment_path = comments_dir.join(&filename);

        if comment_path.exists() {
            return Err(StorageError::FileAlreadyExists(comment_path));
        }

        fs::create_dir_all(&comments_dir)?;
        fs::write(&comment_path, NEW_COMMENT_TEMPLATE)?;

        Ok(comment_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::fs;

    #[rstest]
    fn test_init_new_issue_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let storage = IssueStorage::from_dir(dir.path());

        let result = storage.init_new_issue();
        assert!(result.is_ok());

        let path = result.unwrap();
        assert_eq!(path, dir.path().join("issue.md"));
        assert!(path.exists());

        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(
            content,
            "---\ntitle: \"\"\nlabels: []\nassignees: []\n---\n\nBody\n"
        );
    }

    #[rstest]
    fn test_init_new_issue_returns_error_if_exists() {
        let dir = tempfile::tempdir().unwrap();
        let storage = IssueStorage::from_dir(dir.path());

        // Create the file first
        fs::create_dir_all(dir.path()).unwrap();
        fs::write(dir.path().join("issue.md"), "existing content").unwrap();

        let result = storage.init_new_issue();
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(matches!(err, StorageError::FileAlreadyExists(_)));
    }

    #[rstest]
    fn test_init_new_comment_with_name() {
        let dir = tempfile::tempdir().unwrap();
        let storage = IssueStorage::from_dir(dir.path());

        let result = storage.init_new_comment(Some("my_comment"));
        assert!(result.is_ok());

        let path = result.unwrap();
        assert_eq!(path, dir.path().join("comments/new_my_comment.md"));
        assert!(path.exists());

        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, "Comment body\n");
    }

    #[rstest]
    fn test_init_new_comment_with_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let storage = IssueStorage::from_dir(dir.path());

        let result = storage.init_new_comment(None);
        assert!(result.is_ok());

        let path = result.unwrap();
        let filename = path.file_name().unwrap().to_string_lossy();
        assert!(filename.starts_with("new_"));
        assert!(filename.ends_with(".md"));
        // Check timestamp format: new_YYYYMMDD_HHMMSS.md
        assert!(filename.len() == "new_20240101_120000.md".len());
        assert!(path.exists());
    }

    #[rstest]
    fn test_init_new_comment_returns_error_if_exists() {
        let dir = tempfile::tempdir().unwrap();
        let storage = IssueStorage::from_dir(dir.path());
        let comments_dir = dir.path().join("comments");

        // Create the file first
        fs::create_dir_all(&comments_dir).unwrap();
        fs::write(comments_dir.join("new_existing.md"), "existing content").unwrap();

        let result = storage.init_new_comment(Some("existing"));
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(matches!(err, StorageError::FileAlreadyExists(_)));
    }

    #[rstest]
    fn test_init_new_comment_creates_comments_directory() {
        let dir = tempfile::tempdir().unwrap();
        let storage = IssueStorage::from_dir(dir.path());

        // Ensure comments directory doesn't exist
        assert!(!dir.path().join("comments").exists());

        let result = storage.init_new_comment(Some("test"));
        assert!(result.is_ok());

        // Directory should be created
        assert!(dir.path().join("comments").exists());
    }
}
