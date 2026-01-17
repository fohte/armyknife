use std::fs;
use std::path::Path;

use super::error::{Result, StorageError};
use super::paths::get_issue_dir;
use crate::gh::issue_agent::models::IssueMetadata;

/// Metadata parsed from comment file headers.
/// Format: <!-- key: value -->
#[derive(Debug, Clone, Default)]
pub struct CommentFileMetadata {
    pub author: Option<String>,
    pub created_at: Option<String>,
    pub id: Option<String>,
    pub database_id: Option<i64>,
}

impl CommentFileMetadata {
    /// Parse metadata from header lines.
    /// Each line should be in format: <!-- key: value -->
    fn parse_from_lines(lines: &[&str], path: &Path) -> Result<Self> {
        let mut metadata = Self::default();

        for line in lines {
            if let Some((key, value)) = Self::extract_key_value(line) {
                match key {
                    "author" => metadata.author = Some(value),
                    "createdAt" => metadata.created_at = Some(value),
                    "id" => metadata.id = Some(value),
                    "databaseId" => {
                        metadata.database_id = Some(value.parse().map_err(|_| {
                            StorageError::CommentMetadataParseError {
                                path: path.to_path_buf(),
                                message: format!("Invalid databaseId: {}", value),
                            }
                        })?);
                    }
                    _ => {} // Ignore unknown keys
                }
            }
        }

        Ok(metadata)
    }

    /// Extract key-value pair from a metadata comment line.
    /// Format: <!-- key: value -->
    fn extract_key_value(line: &str) -> Option<(&str, String)> {
        let inner = line.strip_prefix("<!-- ")?.strip_suffix(" -->")?;
        let (key, value) = inner.split_once(": ")?;
        Some((key, value.to_string()))
    }
}

/// A comment read from a local file.
#[derive(Debug, Clone)]
pub struct LocalComment {
    pub filename: String,
    pub metadata: CommentFileMetadata,
    pub body: String,
}

impl LocalComment {
    /// Parse a comment from file content.
    ///
    /// Format:
    /// <!-- author: username -->
    /// <!-- createdAt: 2024-01-01T00:00:00Z -->
    /// <!-- id: node_id -->
    /// <!-- databaseId: 12345 -->
    ///
    /// Body content here...
    pub fn parse(content: &str, filename: String, path: &Path) -> Result<Self> {
        let lines: Vec<&str> = content.lines().collect();

        // Split into header lines (metadata comments) and body lines
        let header_end = lines
            .iter()
            .position(|line| !line.starts_with("<!--") && !line.is_empty())
            .unwrap_or(lines.len());

        let metadata = CommentFileMetadata::parse_from_lines(&lines[..header_end], path)?;

        // Body starts after first empty line following headers, or at first non-metadata line
        let body_start = lines[header_end..]
            .iter()
            .position(|line| !line.is_empty())
            .map(|pos| header_end + pos)
            .unwrap_or(lines.len());

        let body = lines[body_start..].join("\n");

        Ok(Self {
            filename,
            metadata,
            body,
        })
    }

    /// Returns true if this is a new comment (filename starts with "new_").
    pub fn is_new(&self) -> bool {
        self.filename.starts_with("new_")
    }
}

/// Read the issue body from issue.md.
pub fn read_issue_body(repo: &str, issue_number: i64) -> Result<String> {
    let issue_dir = get_issue_dir(repo, issue_number);
    read_issue_body_from_dir(&issue_dir)
}

/// Read the issue body from a specific directory.
pub fn read_issue_body_from_dir(issue_dir: &Path) -> Result<String> {
    let path = issue_dir.join("issue.md");
    if !path.exists() {
        return Err(StorageError::FileNotFound(path));
    }
    let content = fs::read_to_string(&path)?;
    // Trim trailing newline added during save
    Ok(content.trim_end_matches('\n').to_string())
}

/// Read metadata from metadata.json.
pub fn read_metadata(repo: &str, issue_number: i64) -> Result<IssueMetadata> {
    let issue_dir = get_issue_dir(repo, issue_number);
    read_metadata_from_dir(&issue_dir)
}

/// Read metadata from a specific directory.
pub fn read_metadata_from_dir(issue_dir: &Path) -> Result<IssueMetadata> {
    let path = issue_dir.join("metadata.json");
    if !path.exists() {
        return Err(StorageError::FileNotFound(path));
    }
    let content = fs::read_to_string(&path)?;
    let metadata: IssueMetadata = serde_json::from_str(&content)?;
    Ok(metadata)
}

/// Read all comments from the comments/ directory.
pub fn read_comments(repo: &str, issue_number: i64) -> Result<Vec<LocalComment>> {
    let issue_dir = get_issue_dir(repo, issue_number);
    read_comments_from_dir(&issue_dir)
}

/// Read all comments from a specific directory.
pub fn read_comments_from_dir(issue_dir: &Path) -> Result<Vec<LocalComment>> {
    let comments_dir = issue_dir.join("comments");
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

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use tempfile::TempDir;

    fn setup_test_dir() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn test_read_issue_body_from_dir() {
        let dir = setup_test_dir();
        let issue_path = dir.path().join("issue.md");
        fs::write(&issue_path, "Test issue body\n").unwrap();

        let body = read_issue_body_from_dir(dir.path()).unwrap();
        assert_eq!(body, "Test issue body");
    }

    #[test]
    fn test_read_metadata_from_dir() {
        let dir = setup_test_dir();
        let metadata_path = dir.path().join("metadata.json");
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
        fs::write(&metadata_path, metadata_json).unwrap();

        let metadata = read_metadata_from_dir(dir.path()).unwrap();
        assert_eq!(metadata.number, 123);
        assert_eq!(metadata.title, "Test Issue");
        assert_eq!(metadata.state, "OPEN");
    }

    #[test]
    fn test_read_comments_from_dir() {
        let dir = setup_test_dir();
        let comments_dir = dir.path().join("comments");
        fs::create_dir(&comments_dir).unwrap();

        let comment_content = r#"<!-- author: testuser -->
<!-- createdAt: 2024-01-01T00:00:00Z -->
<!-- id: IC_abc123 -->
<!-- databaseId: 12345 -->

This is the comment body."#;

        fs::write(comments_dir.join("001_comment_12345.md"), comment_content).unwrap();

        let comments = read_comments_from_dir(dir.path()).unwrap();
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
        let dir = setup_test_dir();
        let comments_dir = dir.path().join("comments");
        fs::create_dir(&comments_dir).unwrap();

        fs::write(
            comments_dir.join("new_my_comment.md"),
            "New comment content",
        )
        .unwrap();

        let comments = read_comments_from_dir(dir.path()).unwrap();
        assert_eq!(comments.len(), 1);
        assert!(comments[0].is_new());
        assert_eq!(comments[0].body, "New comment content");
    }

    #[rstest]
    #[case("<!-- author: testuser -->", Some(("author", "testuser".to_string())))]
    #[case("<!-- databaseId: 12345 -->", Some(("databaseId", "12345".to_string())))]
    #[case("<!-- id: IC_abc123 -->", Some(("id", "IC_abc123".to_string())))]
    #[case("not a metadata line", None)]
    #[case("<!-- invalid -->", None)]
    #[case("<!-- no-colon-separator -->", None)]
    fn test_extract_metadata_key_value(
        #[case] line: &str,
        #[case] expected: Option<(&str, String)>,
    ) {
        assert_eq!(CommentFileMetadata::extract_key_value(line), expected);
    }
}
