//! Models for new issue creation.

use serde::{Deserialize, Serialize};

/// Frontmatter for new issue creation.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct NewIssueFrontmatter {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub assignees: Vec<String>,
}

/// Parsed new issue from local file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewIssue {
    pub frontmatter: NewIssueFrontmatter,
    pub body: String,
}

impl NewIssue {
    /// Parse issue.md content into NewIssue.
    ///
    /// Expected format:
    /// ```markdown
    /// ---
    /// title: "Issue Title"
    /// labels: []
    /// assignees: []
    /// ---
    ///
    /// Issue body content...
    /// ```
    pub fn parse(content: &str) -> Result<Self, String> {
        let (frontmatter, body) = parse_frontmatter(content)?;

        if frontmatter.title.trim().is_empty() {
            return Err("Title cannot be empty".to_string());
        }

        let body = body.trim().to_string();

        Ok(Self { frontmatter, body })
    }

    /// Get the title from frontmatter.
    pub fn title(&self) -> &str {
        &self.frontmatter.title
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

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use rstest::rstest;

    mod parse_tests {
        use super::*;

        #[rstest]
        fn test_parse_full_format() {
            let content = indoc! {"
                ---
                title: Fix critical bug
                labels:
                  - bug
                  - urgent
                assignees:
                  - fohte
                ---

                This is the issue body.

                Multiple paragraphs are supported.
            "};

            let result = NewIssue::parse(content).unwrap();

            assert_eq!(result.title(), "Fix critical bug");
            assert_eq!(
                result.body,
                indoc! {"
                    This is the issue body.

                    Multiple paragraphs are supported."},
            );
            assert_eq!(
                result.frontmatter.labels,
                vec!["bug".to_string(), "urgent".to_string()]
            );
            assert_eq!(result.frontmatter.assignees, vec!["fohte".to_string()]);
        }

        #[rstest]
        fn test_parse_with_title_only() {
            let content = indoc! {"
                ---
                title: Simple Issue
                ---

                Body text.
            "};

            let result = NewIssue::parse(content).unwrap();

            assert_eq!(result.title(), "Simple Issue");
            assert_eq!(result.body, "Body text.");
            assert!(result.frontmatter.labels.is_empty());
            assert!(result.frontmatter.assignees.is_empty());
        }

        #[rstest]
        fn test_parse_empty_body() {
            let content = indoc! {"
                ---
                title: Title Only
                labels: []
                ---
            "};

            let result = NewIssue::parse(content).unwrap();

            assert_eq!(result.title(), "Title Only");
            assert_eq!(result.body, "");
        }

        #[rstest]
        fn test_parse_inline_labels() {
            let content = indoc! {"
                ---
                title: Inline Style
                labels: [bug, enhancement]
                assignees: [user1, user2]
                ---

                Body.
            "};

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

        #[rstest]
        fn test_parse_body_with_h1_heading() {
            let content = indoc! {"
                ---
                title: Issue Title
                ---

                # Section Heading

                This is the body with an H1 heading.
            "};

            let result = NewIssue::parse(content).unwrap();

            assert_eq!(result.title(), "Issue Title");
            assert_eq!(
                result.body,
                indoc! {"
                    # Section Heading

                    This is the body with an H1 heading."},
            );
        }

        #[rstest]
        #[case::missing_title(
            indoc! {"
                ---
                labels: []
                ---

                Just body without title.
            "},
            "Title cannot be empty"
        )]
        #[case::empty_title(
            indoc! {r#"
                ---
                title: ""
                ---

                Body.
            "#},
            "Title cannot be empty"
        )]
        #[case::whitespace_only_title(
            indoc! {r#"
                ---
                title: "   "
                ---

                Body.
            "#},
            "Title cannot be empty"
        )]
        #[case::unclosed_frontmatter(
            indoc! {"
                ---
                title: Test
                labels: []

                Body.
            "},
            "Unclosed frontmatter"
        )]
        #[case::invalid_yaml(
            indoc! {"
                ---
                title: Test
                labels: [unclosed
                ---

                Body.
            "},
            "Failed to parse frontmatter"
        )]
        #[case::no_frontmatter(
            indoc! {"
                Just body without frontmatter.
            "},
            "Title cannot be empty"
        )]
        fn test_parse_errors(#[case] content: &str, #[case] expected_error: &str) {
            let result = NewIssue::parse(content);
            assert!(result.is_err());
            assert!(
                result.unwrap_err().contains(expected_error),
                "Expected error containing '{expected_error}'"
            );
        }
    }
}
