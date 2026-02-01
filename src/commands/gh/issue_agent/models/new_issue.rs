//! Models for new issue creation.

use serde::{Deserialize, Serialize};

/// Frontmatter for new issue creation.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct NewIssueFrontmatter {
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub assignees: Vec<String>,
}

/// Parsed new issue from local file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewIssue {
    pub frontmatter: NewIssueFrontmatter,
    pub title: String,
    pub body: String,
}

impl NewIssue {
    /// Parse issue.md content into NewIssue.
    ///
    /// Expected format:
    /// ```markdown
    /// ---
    /// labels: []
    /// assignees: []
    /// ---
    ///
    /// # Issue Title
    ///
    /// Issue body content...
    /// ```
    pub fn parse(content: &str) -> Result<Self, String> {
        let (frontmatter, rest) = parse_frontmatter(content)?;
        let (title, body) = parse_title_and_body(rest)?;

        Ok(Self {
            frontmatter,
            title,
            body,
        })
    }
}

/// Parse YAML frontmatter from content.
/// Returns (frontmatter, remaining content after frontmatter).
fn parse_frontmatter(content: &str) -> Result<(NewIssueFrontmatter, &str), String> {
    let content = content.trim_start();

    // Check for frontmatter delimiter
    if !content.starts_with("---") {
        // No frontmatter, use defaults
        return Ok((NewIssueFrontmatter::default(), content));
    }

    // Find the closing delimiter
    let after_first = &content[3..];
    let end_pos = after_first
        .find("\n---")
        .ok_or("Unclosed frontmatter: missing closing '---'")?;

    let yaml_content = &after_first[..end_pos].trim();
    let rest = &after_first[end_pos + 4..]; // Skip "\n---"

    // Parse YAML
    let frontmatter: NewIssueFrontmatter = if yaml_content.is_empty() {
        NewIssueFrontmatter::default()
    } else {
        serde_yaml::from_str(yaml_content)
            .map_err(|e| format!("Failed to parse frontmatter YAML: {e}"))?
    };

    Ok((frontmatter, rest))
}

/// Parse title (first H1 heading) and body from content.
fn parse_title_and_body(content: &str) -> Result<(String, String), String> {
    let content = content.trim_start_matches('\n');

    // Find title line (first line starting with "# ")
    let mut lines = content.lines();
    let mut title = None;
    let mut body_start_idx = 0;

    for line in lines.by_ref() {
        // Support both "# Title" and "#" (empty title, which will be rejected later)
        if line.starts_with('#') {
            let t = line
                .strip_prefix("# ")
                .or_else(|| line.strip_prefix("#"))
                .unwrap_or("");
            title = Some(t.trim().to_string());
            body_start_idx = content.find(line).unwrap_or(0) + line.len();
            break;
        }
        // Skip empty lines before title
        if !line.trim().is_empty() {
            return Err(format!(
                "Expected title line starting with '# ', found: '{line}'"
            ));
        }
    }

    let title = title.ok_or("Missing title: expected a line starting with '# '")?;

    if title.is_empty() {
        return Err("Title cannot be empty".to_string());
    }

    // Rest is body (skip leading newlines)
    let body = content[body_start_idx..]
        .trim_start_matches('\n')
        .to_string();

    Ok((title, body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    mod parse_tests {
        use super::*;

        #[rstest]
        fn test_parse_full_format() {
            let content = r#"---
labels:
  - bug
  - urgent
assignees:
  - fohte
---

# Fix critical bug

This is the issue body.

Multiple paragraphs are supported.
"#;

            let result = NewIssue::parse(content).unwrap();

            assert_eq!(result.title, "Fix critical bug");
            assert_eq!(
                result.body,
                "This is the issue body.\n\nMultiple paragraphs are supported.\n"
            );
            assert_eq!(
                result.frontmatter.labels,
                vec!["bug".to_string(), "urgent".to_string()]
            );
            assert_eq!(result.frontmatter.assignees, vec!["fohte".to_string()]);
        }

        #[rstest]
        fn test_parse_empty_frontmatter() {
            let content = r#"---
---

# Simple Issue

Body text.
"#;

            let result = NewIssue::parse(content).unwrap();

            assert_eq!(result.title, "Simple Issue");
            assert_eq!(result.body, "Body text.\n");
            assert!(result.frontmatter.labels.is_empty());
            assert!(result.frontmatter.assignees.is_empty());
        }

        #[rstest]
        fn test_parse_no_frontmatter() {
            let content = r#"# Issue Without Frontmatter

Just the body.
"#;

            let result = NewIssue::parse(content).unwrap();

            assert_eq!(result.title, "Issue Without Frontmatter");
            assert_eq!(result.body, "Just the body.\n");
            assert!(result.frontmatter.labels.is_empty());
            assert!(result.frontmatter.assignees.is_empty());
        }

        #[rstest]
        fn test_parse_empty_body() {
            let content = r#"---
labels: []
---

# Title Only
"#;

            let result = NewIssue::parse(content).unwrap();

            assert_eq!(result.title, "Title Only");
            assert_eq!(result.body, "");
        }

        #[rstest]
        fn test_parse_missing_title() {
            let content = r#"---
labels: []
---

Just body without title.
"#;

            let result = NewIssue::parse(content);
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("Expected title line"));
        }

        #[rstest]
        fn test_parse_empty_title() {
            let content = r#"---
---

#

Body.
"#;

            let result = NewIssue::parse(content);
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("Title cannot be empty"));
        }

        #[rstest]
        fn test_parse_unclosed_frontmatter() {
            let content = r#"---
labels: []

# Title

Body.
"#;

            let result = NewIssue::parse(content);
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("Unclosed frontmatter"));
        }

        #[rstest]
        fn test_parse_invalid_yaml() {
            let content = r#"---
labels: [unclosed
---

# Title

Body.
"#;

            let result = NewIssue::parse(content);
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("Failed to parse frontmatter"));
        }

        #[rstest]
        fn test_parse_inline_labels() {
            let content = r#"---
labels: [bug, enhancement]
assignees: [user1, user2]
---

# Inline Style

Body.
"#;

            let result = NewIssue::parse(content).unwrap();

            assert_eq!(
                result.frontmatter.labels,
                vec!["bug".to_string(), "enhancement".to_string()]
            );
            assert_eq!(
                result.frontmatter.assignees,
                vec!["user1".to_string(), "user2".to_string()]
            );
        }
    }
}
