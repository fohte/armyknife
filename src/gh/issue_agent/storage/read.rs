use std::path::Path;

use super::error::{Result, StorageError};

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

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

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
