//! Models for new issue creation.

use serde::{Deserialize, Serialize};

use super::editable::EditableIssueFields;

/// Frontmatter for new issue creation.
///
/// Shares editable fields with [`super::IssueFrontmatter`] via `EditableIssueFields`,
/// so adding a field there propagates to both create and edit paths automatically.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct NewIssueFrontmatter {
    #[serde(flatten)]
    pub fields: EditableIssueFields,
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
    /// parentIssue: owner/repo#5
    /// subIssues:
    ///   - owner/repo#10
    /// ---
    ///
    /// Issue body content...
    /// ```
    pub fn parse(content: &str) -> Result<Self, String> {
        let (frontmatter, body) = parse_frontmatter(content)?;

        if frontmatter.fields.title.trim().is_empty() {
            return Err("Title cannot be empty".to_string());
        }

        let body = body.trim().to_string();

        Ok(Self { frontmatter, body })
    }

    /// Get the title from frontmatter.
    pub fn title(&self) -> &str {
        &self.frontmatter.fields.title
    }

    /// Get the labels from frontmatter.
    pub fn labels(&self) -> &[String] {
        &self.frontmatter.fields.labels
    }

    /// Get the assignees from frontmatter.
    pub fn assignees(&self) -> &[String] {
        &self.frontmatter.fields.assignees
    }

    /// Get the parent issue reference from frontmatter, if any.
    pub fn parent_issue(&self) -> Option<&str> {
        self.frontmatter.fields.parent_issue.as_deref()
    }

    /// Get the sub-issue references from frontmatter.
    pub fn sub_issues(&self) -> &[String] {
        &self.frontmatter.fields.sub_issues
    }
}

/// Parse YAML frontmatter from content.
/// Returns (frontmatter, remaining content after frontmatter).
///
/// Rejects unknown top-level keys with a descriptive error so that typos like
/// `parentIssues` (vs `parentIssue`) surface immediately rather than being
/// silently dropped.
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

    let yaml_content = after_first[..end_pos].trim();
    let rest = &after_first[end_pos + 4..]; // Skip "\n---"

    // Parse YAML
    let frontmatter: NewIssueFrontmatter = if yaml_content.is_empty() {
        NewIssueFrontmatter::default()
    } else {
        validate_known_keys(yaml_content)?;
        serde_yaml::from_str(yaml_content)
            .map_err(|e| format!("Failed to parse frontmatter YAML: {e}"))?
    };

    Ok((frontmatter, rest))
}

/// Reject unknown top-level YAML keys.
///
/// `serde(flatten)` is incompatible with `serde(deny_unknown_fields)`, so we
/// validate the key set manually before deserializing into the typed struct.
fn validate_known_keys(yaml: &str) -> Result<(), String> {
    use std::collections::BTreeSet;

    let value: serde_yaml::Value =
        serde_yaml::from_str(yaml).map_err(|e| format!("Failed to parse frontmatter YAML: {e}"))?;

    let mapping = match value {
        serde_yaml::Value::Mapping(m) => m,
        serde_yaml::Value::Null => return Ok(()),
        _ => return Err("Frontmatter must be a YAML mapping".to_string()),
    };

    let known: BTreeSet<&str> = EditableIssueFields::KNOWN_KEYS.iter().copied().collect();
    let mut unknown: Vec<String> = Vec::new();
    for (key, _) in &mapping {
        if let Some(name) = key.as_str()
            && !known.contains(name)
        {
            unknown.push(name.to_string());
        }
    }

    if unknown.is_empty() {
        Ok(())
    } else {
        unknown.sort();
        Err(format!(
            "Unknown frontmatter key(s): {}. Allowed keys: {}",
            unknown.join(", "),
            EditableIssueFields::KNOWN_KEYS.join(", ")
        ))
    }
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
                result.labels(),
                vec!["bug".to_string(), "urgent".to_string()]
            );
            assert_eq!(result.assignees(), vec!["fohte".to_string()]);
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
            assert!(result.labels().is_empty());
            assert!(result.assignees().is_empty());
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
                result.labels(),
                vec!["bug".to_string(), "enhancement".to_string()]
            );
            assert_eq!(
                result.assignees(),
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
        fn test_parse_with_parent_issue() {
            let content = indoc! {"
                ---
                title: Child Issue
                parentIssue: owner/repo#42
                ---

                Body.
            "};

            let result = NewIssue::parse(content).unwrap();

            assert_eq!(result.title(), "Child Issue");
            assert_eq!(result.parent_issue(), Some("owner/repo#42"));
            assert!(result.sub_issues().is_empty());
        }

        #[rstest]
        fn test_parse_with_sub_issues() {
            let content = indoc! {"
                ---
                title: Parent Issue
                subIssues:
                  - owner/repo#10
                  - owner/repo#11
                ---

                Body.
            "};

            let result = NewIssue::parse(content).unwrap();

            assert_eq!(result.title(), "Parent Issue");
            assert_eq!(
                result.sub_issues(),
                vec!["owner/repo#10".to_string(), "owner/repo#11".to_string()]
            );
            assert!(result.parent_issue().is_none());
        }

        #[rstest]
        fn test_parse_with_parent_and_sub_issues() {
            let content = indoc! {"
                ---
                title: Middle Issue
                parentIssue: owner/repo#1
                subIssues:
                  - owner/repo#100
                ---

                Body.
            "};

            let result = NewIssue::parse(content).unwrap();

            assert_eq!(result.parent_issue(), Some("owner/repo#1"));
            assert_eq!(result.sub_issues(), vec!["owner/repo#100".to_string()]);
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
            "Unclosed frontmatter: missing closing '---'"
        )]
        #[case::no_frontmatter(
            indoc! {"
                Just body without frontmatter.
            "},
            "Title cannot be empty"
        )]
        #[case::unknown_key_typo(
            indoc! {"
                ---
                title: Typo Issue
                parentIssues: owner/repo#5
                ---

                Body.
            "},
            "Unknown frontmatter key(s): parentIssues. \
             Allowed keys: title, labels, assignees, milestone, parentIssue, subIssues"
        )]
        #[case::unknown_key_arbitrary(
            indoc! {"
                ---
                title: Arbitrary Key
                random_key: value
                ---

                Body.
            "},
            "Unknown frontmatter key(s): random_key. \
             Allowed keys: title, labels, assignees, milestone, parentIssue, subIssues"
        )]
        fn test_parse_errors(#[case] content: &str, #[case] expected_error: &str) {
            let err = NewIssue::parse(content).unwrap_err();
            assert_eq!(err, expected_error);
        }

        /// Invalid YAML content from the user goes through serde_yaml, whose
        /// error message we don't want to pin to a specific format. Verify it
        /// is wrapped with our prefix instead.
        #[rstest]
        fn test_parse_invalid_yaml_is_prefixed() {
            let content = indoc! {"
                ---
                title: Test
                labels: [unclosed
                ---

                Body.
            "};
            let err = NewIssue::parse(content).unwrap_err();
            assert!(
                err.starts_with("Failed to parse frontmatter YAML:"),
                "expected prefix, got: {err}"
            );
        }
    }
}
