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
    ///
    /// This function will:
    /// 1. Save all comments from GitHub
    /// 2. Remove stale comment files (files for comments that no longer exist on GitHub,
    ///    or files with old indices for comments that were re-indexed)
    /// 3. Preserve `new_*.md` files (local comments not yet pushed to GitHub)
    pub fn save_comments(&self, comments: &[Comment]) -> Result<()> {
        let comments_dir = self.dir.join("comments");
        fs::create_dir_all(&comments_dir)?;

        // Collect filenames that will be created
        let mut saved_filenames: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for (i, comment) in comments.iter().enumerate() {
            let index = format!("{:03}", i + 1);
            let filename = format!("{}_comment_{}.md", index, comment.database_id);
            let path = comments_dir.join(&filename);

            let content = LocalComment::format_from_comment(comment);
            fs::write(&path, content)?;
            saved_filenames.insert(filename);
        }

        // Remove stale comment files (but preserve new_*.md files)
        self.remove_stale_comment_files(&comments_dir, &saved_filenames)?;

        Ok(())
    }

    /// Remove stale comment files from the comments directory.
    ///
    /// A file is considered stale if:
    /// - It matches the pattern `{index}_comment_{database_id}.md`
    /// - Its filename is not in the set of saved_filenames
    ///
    /// Files matching `new_*.md` are preserved (local comments not yet pushed).
    fn remove_stale_comment_files(
        &self,
        comments_dir: &std::path::Path,
        saved_filenames: &std::collections::HashSet<String>,
    ) -> Result<()> {
        let entries = match fs::read_dir(comments_dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };

        for entry in entries {
            let entry = entry?;
            let filename = entry.file_name();
            let filename_str = filename.to_string_lossy();

            // Skip new_*.md files (local comments not yet pushed)
            if filename_str.starts_with("new_") {
                continue;
            }

            // Check if this is a comment file and not in saved_filenames
            if Self::parse_database_id_from_filename(&filename_str).is_some()
                && !saved_filenames.contains(filename_str.as_ref())
            {
                fs::remove_file(entry.path())?;
            }
        }

        Ok(())
    }

    /// Parse database_id from a comment filename.
    ///
    /// Expected format: `{index}_comment_{database_id}.md`
    /// Returns None if the filename doesn't match the expected format.
    fn parse_database_id_from_filename(filename: &str) -> Option<i64> {
        // Pattern: XXX_comment_YYYYY.md where XXX is index and YYYYY is database_id
        let stripped = filename.strip_suffix(".md")?;
        let parts: Vec<&str> = stripped.split("_comment_").collect();
        if parts.len() != 2 {
            return None;
        }
        parts[1].parse().ok()
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

    #[test]
    fn test_save_comments_removes_stale_files() {
        let dir = tempfile::tempdir().unwrap();
        let storage = IssueStorage::from_dir(dir.path());
        let comments_dir = dir.path().join("comments");
        fs::create_dir_all(&comments_dir).unwrap();

        // Create initial comments: comment_11111 and comment_22222
        let initial_comments = vec![
            Comment {
                id: "IC_11111".to_string(),
                database_id: 11111,
                author: Some(Author {
                    login: "user1".to_string(),
                }),
                created_at: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
                body: "First".to_string(),
            },
            Comment {
                id: "IC_22222".to_string(),
                database_id: 22222,
                author: Some(Author {
                    login: "user2".to_string(),
                }),
                created_at: Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap(),
                body: "Second".to_string(),
            },
        ];
        storage.save_comments(&initial_comments).unwrap();
        assert!(comments_dir.join("001_comment_11111.md").exists());
        assert!(comments_dir.join("002_comment_22222.md").exists());

        // Now save only comment_22222 (simulating comment_11111 being deleted on GitHub)
        let updated_comments = vec![Comment {
            id: "IC_22222".to_string(),
            database_id: 22222,
            author: Some(Author {
                login: "user2".to_string(),
            }),
            created_at: Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap(),
            body: "Second".to_string(),
        }];
        storage.save_comments(&updated_comments).unwrap();

        // Stale file should be removed, remaining file should be re-indexed
        assert!(
            !comments_dir.join("001_comment_11111.md").exists(),
            "Stale comment file should be deleted"
        );
        assert!(
            !comments_dir.join("002_comment_22222.md").exists(),
            "Old index file should be replaced with new index"
        );
        assert!(
            comments_dir.join("001_comment_22222.md").exists(),
            "Remaining comment should be re-indexed to 001"
        );
    }

    #[test]
    fn test_save_comments_preserves_new_files() {
        let dir = tempfile::tempdir().unwrap();
        let storage = IssueStorage::from_dir(dir.path());
        let comments_dir = dir.path().join("comments");
        fs::create_dir_all(&comments_dir).unwrap();

        // Create a new_*.md file (user's new comment not yet pushed)
        fs::write(comments_dir.join("new_my_comment.md"), "My new comment").unwrap();

        // Save comments from GitHub
        let comments = vec![Comment {
            id: "IC_12345".to_string(),
            database_id: 12345,
            author: Some(Author {
                login: "testuser".to_string(),
            }),
            created_at: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            body: "Remote comment".to_string(),
        }];
        storage.save_comments(&comments).unwrap();

        // new_*.md file should be preserved
        assert!(
            comments_dir.join("new_my_comment.md").exists(),
            "new_*.md files should be preserved"
        );
        assert!(comments_dir.join("001_comment_12345.md").exists());
    }

    #[test]
    fn test_parse_database_id_from_filename() {
        assert_eq!(
            IssueStorage::parse_database_id_from_filename("001_comment_12345.md"),
            Some(12345)
        );
        assert_eq!(
            IssueStorage::parse_database_id_from_filename("999_comment_99999999.md"),
            Some(99999999)
        );
        assert_eq!(
            IssueStorage::parse_database_id_from_filename("new_my_comment.md"),
            None
        );
        assert_eq!(
            IssueStorage::parse_database_id_from_filename("invalid.md"),
            None
        );
        assert_eq!(
            IssueStorage::parse_database_id_from_filename("001_comment_notanumber.md"),
            None
        );
    }
}
