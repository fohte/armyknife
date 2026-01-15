use std::fs;
use std::path::Path;

use super::error::Result;
use super::paths::get_issue_dir;
use crate::gh::issue_agent::models::{Comment, IssueMetadata};

/// Save the issue body to issue.md.
pub fn save_issue_body(repo: &str, issue_number: i64, body: &str) -> Result<()> {
    let issue_dir = get_issue_dir(repo, issue_number);
    save_issue_body_to_dir(&issue_dir, body)
}

/// Save the issue body to a specific directory.
pub fn save_issue_body_to_dir(issue_dir: &Path, body: &str) -> Result<()> {
    fs::create_dir_all(issue_dir)?;
    let path = issue_dir.join("issue.md");
    fs::write(&path, format!("{}\n", body))?;
    Ok(())
}

/// Save metadata to metadata.json.
pub fn save_metadata(repo: &str, issue_number: i64, metadata: &IssueMetadata) -> Result<()> {
    let issue_dir = get_issue_dir(repo, issue_number);
    save_metadata_to_dir(&issue_dir, metadata)
}

/// Save metadata to a specific directory.
pub fn save_metadata_to_dir(issue_dir: &Path, metadata: &IssueMetadata) -> Result<()> {
    fs::create_dir_all(issue_dir)?;
    let path = issue_dir.join("metadata.json");
    let json = serde_json::to_string_pretty(metadata)?;
    fs::write(&path, json)?;
    Ok(())
}

/// Save comments to the comments/ directory.
pub fn save_comments(repo: &str, issue_number: i64, comments: &[Comment]) -> Result<()> {
    let issue_dir = get_issue_dir(repo, issue_number);
    save_comments_to_dir(&issue_dir, comments)
}

/// Save comments to a specific directory.
pub fn save_comments_to_dir(issue_dir: &Path, comments: &[Comment]) -> Result<()> {
    let comments_dir = issue_dir.join("comments");
    fs::create_dir_all(&comments_dir)?;

    for (i, comment) in comments.iter().enumerate() {
        let index = format!("{:03}", i + 1);
        let filename = format!("{}_comment_{}.md", index, comment.database_id);
        let path = comments_dir.join(&filename);

        let author = comment
            .author
            .as_ref()
            .map(|a| a.login.as_str())
            .unwrap_or("unknown");
        let created_at = comment.created_at.to_rfc3339();
        let id = &comment.id;
        let database_id = comment.database_id;

        let content = format!(
            "<!-- author: {} -->\n<!-- createdAt: {} -->\n<!-- id: {} -->\n<!-- databaseId: {} -->\n\n{}",
            author, created_at, id, database_id, comment.body
        );

        fs::write(&path, content)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gh::issue_agent::models::Author;
    use chrono::{TimeZone, Utc};
    use tempfile::TempDir;

    fn setup_test_dir() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn test_save_issue_body_to_dir() {
        let dir = setup_test_dir();
        save_issue_body_to_dir(dir.path(), "Test body content").unwrap();

        let content = fs::read_to_string(dir.path().join("issue.md")).unwrap();
        assert_eq!(content, "Test body content\n");
    }

    #[test]
    fn test_save_metadata_to_dir() {
        let dir = setup_test_dir();
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

        save_metadata_to_dir(dir.path(), &metadata).unwrap();

        let content = fs::read_to_string(dir.path().join("metadata.json")).unwrap();
        let loaded: IssueMetadata = serde_json::from_str(&content).unwrap();
        assert_eq!(loaded.number, 123);
        assert_eq!(loaded.title, "Test Issue");
    }

    #[test]
    fn test_save_comments_to_dir() {
        let dir = setup_test_dir();
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

        save_comments_to_dir(dir.path(), &comments).unwrap();

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
