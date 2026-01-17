use std::fs;

use super::error::Result;
use super::issue_storage::IssueStorage;
use super::read::LocalComment;
use crate::gh::issue_agent::models::{Comment, IssueMetadata};

impl IssueStorage {
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

            let content = LocalComment::format_from_comment(comment);
            fs::write(&path, content)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gh::issue_agent::models::Author;
    use chrono::{TimeZone, Utc};
    use std::fs;

    #[test]
    fn test_save_body() {
        let dir = tempfile::tempdir().unwrap();
        let storage = IssueStorage::from_dir(dir.path());
        storage.save_body("Test body content").unwrap();

        let content = fs::read_to_string(dir.path().join("issue.md")).unwrap();
        assert_eq!(content, "Test body content\n");
    }

    #[test]
    fn test_save_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let storage = IssueStorage::from_dir(dir.path());
        let metadata = IssueMetadata {
            number: 123,
            title: "Test Issue".to_string(),
            state: "OPEN".to_string(),
            labels: vec!["bug".to_string()],
            assignees: vec!["user1".to_string()],
            milestone: None,
            author: "author1".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-02T00:00:00Z".to_string(),
        };

        storage.save_metadata(&metadata).unwrap();

        let content = fs::read_to_string(dir.path().join("metadata.json")).unwrap();
        let loaded: IssueMetadata = serde_json::from_str(&content).unwrap();
        assert_eq!(loaded.number, 123);
        assert_eq!(loaded.title, "Test Issue");
    }

    #[test]
    fn test_save_comments() {
        let dir = tempfile::tempdir().unwrap();
        let storage = IssueStorage::from_dir(dir.path());
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

        let comments_dir = dir.path().join("comments");
        assert!(comments_dir.exists());

        let first_comment = fs::read_to_string(comments_dir.join("001_comment_12345.md")).unwrap();
        assert!(first_comment.contains("<!-- author: testuser -->"));
        assert!(first_comment.contains("<!-- databaseId: 12345 -->"));
        assert!(first_comment.contains("First comment"));

        let second_comment = fs::read_to_string(comments_dir.join("002_comment_67890.md")).unwrap();
        assert!(second_comment.contains("<!-- author: unknown -->"));
        assert!(second_comment.contains("Second comment"));
    }
}
