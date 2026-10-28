use std::path::Path;

use indoc::formatdoc;

use super::error::{Result, StorageError};
use crate::commands::gh::issue_agent::models::Comment;

/// Metadata parsed from comment file headers.
/// Format: <!-- key: value -->
#[derive(Debug, Clone, Default)]
pub struct CommentFileMetadata {
    pub author: Option<String>,
    pub created_at: Option<String>,
    /// Timestamp when the comment was last updated (for conflict detection).
    pub updated_at: Option<String>,
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
                    "updatedAt" => metadata.updated_at = Some(value),
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

        let header_end = lines
            .iter()
            .take_while(|line| line.starts_with("<!--"))
            .count();

        let metadata = CommentFileMetadata::parse_from_lines(&lines[..header_end], path)?;

        // The separator distinguishes metadata comments from HTML comments in the body.
        let body_start = if lines.get(header_end) == Some(&"") {
            header_end + 1
        } else {
            header_end
        };

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

    /// Format a Comment as file content with metadata headers.
    pub fn format_from_comment(comment: &Comment) -> String {
        let author = comment
            .author
            .as_ref()
            .map(|a| a.login.as_str())
            .unwrap_or("unknown");
        let created_at = comment.created_at.to_rfc3339();
        let updated_at = comment.updated_at.to_rfc3339();

        formatdoc! {"
            <!-- author: {} -->
            <!-- createdAt: {} -->
            <!-- updatedAt: {} -->
            <!-- id: {} -->
            <!-- databaseId: {} -->

            {}
        ", author, created_at, updated_at, comment.id, comment.database_id, comment.body.trim()}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use indoc::indoc;
    use rstest::{fixture, rstest};

    #[fixture]
    fn comment() -> Comment {
        let timestamp = Utc.with_ymd_and_hms(2031, 4, 5, 6, 7, 8).unwrap();
        Comment {
            id: "IC_sample".to_string(),
            database_id: 17,
            author: Some(crate::commands::gh::issue_agent::models::Author {
                login: "sample-author".to_string(),
            }),
            created_at: timestamp,
            updated_at: timestamp,
            body: "Comment text".to_string(),
        }
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

    #[rstest]
    #[case::single_html_comment(indoc!("<!-- sticky marker -->"))]
    #[case::html_comment_before_text(indoc! {"
        <!-- sticky marker -->
        Comment text"})]
    #[case::html_comment_before_blank_line(indoc! {"
        <!-- sticky marker -->

        Comment text"})]
    fn test_format_and_parse_preserve_body_starting_with_html_comment(
        #[case] body: &str,
        mut comment: Comment,
    ) {
        comment.body = body.to_string();

        let content = LocalComment::format_from_comment(&comment);
        let parsed =
            LocalComment::parse(&content, "comment.md".to_string(), Path::new("comment.md"))
                .unwrap();

        assert_eq!(
            (
                parsed.metadata.author,
                parsed.metadata.created_at,
                parsed.metadata.updated_at,
                parsed.metadata.id,
                parsed.metadata.database_id,
                parsed.body,
            ),
            (
                Some("sample-author".to_string()),
                Some(comment.created_at.to_rfc3339()),
                Some(comment.updated_at.to_rfc3339()),
                Some("IC_sample".to_string()),
                Some(17),
                body.to_string(),
            ),
        );
    }

    #[rstest]
    fn test_parse_preserves_extra_blank_line_in_body() {
        let content = indoc! {"
            <!-- author: sample-author -->
            <!-- createdAt: 2031-04-05T06:07:08+00:00 -->
            <!-- updatedAt: 2031-04-05T06:07:08+00:00 -->
            <!-- id: IC_sample -->
            <!-- databaseId: 17 -->


            Comment text"};
        let parsed =
            LocalComment::parse(content, "comment.md".to_string(), Path::new("comment.md"))
                .unwrap();

        assert_eq!(
            (
                parsed.metadata.author,
                parsed.metadata.created_at,
                parsed.metadata.updated_at,
                parsed.metadata.id,
                parsed.metadata.database_id,
                parsed.body,
            ),
            (
                Some("sample-author".to_string()),
                Some("2031-04-05T06:07:08+00:00".to_string()),
                Some("2031-04-05T06:07:08+00:00".to_string()),
                Some("IC_sample".to_string()),
                Some(17),
                indoc! {"

                    Comment text"}
                .to_string(),
            ),
        );
    }
}
