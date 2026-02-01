use std::fs;

use super::error::{Result, StorageError};
use super::issue_storage::IssueStorage;
use super::read::LocalComment;
use crate::commands::gh::issue_agent::models::{IssueFrontmatter, IssueMetadata};

/// Parsed issue content from issue.md.
#[derive(Debug, Clone)]
pub struct IssueContent {
    pub frontmatter: IssueFrontmatter,
    pub body: String,
}

impl IssueStorage {
    /// Read and parse issue.md with frontmatter.
    #[cfg(test)]
    pub fn read_issue(&self) -> Result<IssueContent> {
        let path = self.dir.join("issue.md");
        if !path.exists() {
            return Err(StorageError::FileNotFound(path));
        }
        let content = fs::read_to_string(&path)?;
        parse_issue_md(&content)
    }

    /// Read metadata from issue.md frontmatter.
    /// Falls back to metadata.json for backward compatibility.
    pub fn read_metadata(&self) -> Result<IssueMetadata> {
        // Try reading from issue.md frontmatter first
        let issue_path = self.dir.join("issue.md");
        if issue_path.exists() {
            let content = fs::read_to_string(&issue_path)?;
            if let Ok(issue_content) = parse_issue_md(&content) {
                return Ok(issue_content.frontmatter.into());
            }
        }

        // Fall back to legacy metadata.json
        let metadata_path = self.dir.join("metadata.json");
        if !metadata_path.exists() {
            return Err(StorageError::FileNotFound(metadata_path));
        }
        let content = fs::read_to_string(&metadata_path)?;
        let metadata: IssueMetadata = serde_json::from_str(&content)?;
        Ok(metadata)
    }

    /// Read the issue body from issue.md.
    /// Handles both frontmatter and legacy (body-only) formats.
    pub fn read_body(&self) -> Result<String> {
        let path = self.dir.join("issue.md");
        if !path.exists() {
            return Err(StorageError::FileNotFound(path));
        }
        let content = fs::read_to_string(&path)?;

        // Try parsing with frontmatter first
        if let Ok(issue_content) = parse_issue_md(&content) {
            return Ok(issue_content.body);
        }

        // Fall back to legacy format (body only)
        Ok(content.trim_end_matches('\n').to_string())
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

/// Parse issue.md content with YAML frontmatter.
fn parse_issue_md(content: &str) -> Result<IssueContent> {
    let content = content.trim_start();

    // Check for frontmatter delimiter
    if !content.starts_with("---") {
        return Err(StorageError::ParseError(
            "issue.md missing frontmatter".to_string(),
        ));
    }

    // Find the closing delimiter
    let after_first = &content[3..];
    let end_pos = after_first
        .find("\n---")
        .ok_or_else(|| StorageError::ParseError("Unclosed frontmatter in issue.md".to_string()))?;

    let yaml_content = &after_first[..end_pos];
    let rest = &after_first[end_pos + 4..]; // Skip "\n---"

    // Parse YAML frontmatter
    let frontmatter: IssueFrontmatter = serde_yaml::from_str(yaml_content)
        .map_err(|e| StorageError::ParseError(format!("Failed to parse frontmatter YAML: {e}")))?;

    // Rest is body (skip leading newlines, trim trailing newline)
    let body = rest
        .trim_start_matches('\n')
        .trim_end_matches('\n')
        .to_string();

    Ok(IssueContent { frontmatter, body })
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use std::fs;

    #[test]
    fn test_read_issue_with_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let storage = IssueStorage::from_dir(dir.path());
        let issue_md = indoc! {r#"
            ---
            title: Test Issue
            labels:
              - bug
            assignees:
              - user1
            milestone: null
            readonly:
              number: 123
              state: OPEN
              author: author1
              createdAt: "2024-01-01T00:00:00Z"
              updatedAt: "2024-01-02T00:00:00Z"
            ---

            Test issue body
        "#};
        fs::write(dir.path().join("issue.md"), issue_md).unwrap();

        let issue = storage.read_issue().unwrap();
        assert_eq!(issue.frontmatter.title, "Test Issue");
        assert_eq!(issue.frontmatter.labels, vec!["bug"]);
        assert_eq!(issue.frontmatter.assignees, vec!["user1"]);
        assert_eq!(issue.frontmatter.readonly.number, 123);
        assert_eq!(issue.body, "Test issue body");
    }

    #[test]
    fn test_read_body_with_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let storage = IssueStorage::from_dir(dir.path());
        let issue_md = indoc! {r#"
            ---
            title: Test Issue
            labels: []
            assignees: []
            readonly:
              number: 123
              state: OPEN
              author: author1
              createdAt: "2024-01-01T00:00:00Z"
              updatedAt: "2024-01-02T00:00:00Z"
            ---

            Test issue body
        "#};
        fs::write(dir.path().join("issue.md"), issue_md).unwrap();

        let body = storage.read_body().unwrap();
        assert_eq!(body, "Test issue body");
    }

    #[test]
    fn test_read_body_legacy_format() {
        let dir = tempfile::tempdir().unwrap();
        let storage = IssueStorage::from_dir(dir.path());
        fs::write(dir.path().join("issue.md"), "Legacy body content\n").unwrap();

        let body = storage.read_body().unwrap();
        assert_eq!(body, "Legacy body content");
    }

    #[test]
    fn test_read_metadata_from_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let storage = IssueStorage::from_dir(dir.path());
        let issue_md = indoc! {r#"
            ---
            title: Test Issue
            labels:
              - bug
            assignees:
              - user1
            milestone: null
            readonly:
              number: 123
              state: OPEN
              author: author1
              createdAt: "2024-01-01T00:00:00Z"
              updatedAt: "2024-01-02T00:00:00Z"
            ---

            Body
        "#};
        fs::write(dir.path().join("issue.md"), issue_md).unwrap();

        let metadata = storage.read_metadata().unwrap();
        assert_eq!(metadata.number, 123);
        assert_eq!(metadata.title, "Test Issue");
        assert_eq!(metadata.state, "OPEN");
    }

    #[test]
    fn test_read_metadata_fallback_to_json() {
        let dir = tempfile::tempdir().unwrap();
        let storage = IssueStorage::from_dir(dir.path());

        // Legacy format: body only in issue.md
        fs::write(dir.path().join("issue.md"), "Body only\n").unwrap();

        // metadata.json exists
        let metadata_json = indoc! {r#"
            {
                "number": 123,
                "title": "Test Issue",
                "state": "OPEN",
                "labels": ["bug"],
                "assignees": ["user1"],
                "milestone": null,
                "author": "author1",
                "createdAt": "2024-01-01T00:00:00Z",
                "updatedAt": "2024-01-02T00:00:00Z"
            }
        "#};
        fs::write(dir.path().join("metadata.json"), metadata_json).unwrap();

        let metadata = storage.read_metadata().unwrap();
        assert_eq!(metadata.number, 123);
        assert_eq!(metadata.title, "Test Issue");
    }

    #[test]
    fn test_read_comments() {
        let dir = tempfile::tempdir().unwrap();
        let storage = IssueStorage::from_dir(dir.path());
        let comments_dir = dir.path().join("comments");
        fs::create_dir(&comments_dir).unwrap();

        let comment_content = indoc! {"
            <!-- author: testuser -->
            <!-- createdAt: 2024-01-01T00:00:00Z -->
            <!-- id: IC_abc123 -->
            <!-- databaseId: 12345 -->

            This is the comment body.
        "};
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

    #[test]
    fn test_read_new_comment() {
        let dir = tempfile::tempdir().unwrap();
        let storage = IssueStorage::from_dir(dir.path());
        let comments_dir = dir.path().join("comments");
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
