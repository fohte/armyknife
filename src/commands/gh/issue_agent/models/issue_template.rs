//! Models for GitHub Issue Templates.

use serde::{Deserialize, Serialize};

/// GitHub Issue Template fetched from the API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IssueTemplate {
    /// Template name (required)
    pub name: String,
    /// Default issue title
    pub title: Option<String>,
    /// Template body content
    pub body: Option<String>,
    /// Template description
    pub about: Option<String>,
    /// Template filename
    pub filename: Option<String>,
    /// Default labels
    #[serde(default)]
    pub labels: Vec<String>,
    /// Default assignees
    #[serde(default)]
    pub assignees: Vec<String>,
}

impl IssueTemplate {
    /// Convert to issue.md content (frontmatter + body).
    ///
    /// The output format matches the issue-agent's expected format:
    /// ```markdown
    /// ---
    /// title: "Default Title"
    /// labels: [label1, label2]
    /// assignees: [user1, user2]
    /// ---
    ///
    /// Body content
    /// ```
    pub fn to_issue_content(&self) -> String {
        #[derive(Serialize)]
        struct Frontmatter<'a> {
            title: &'a str,
            labels: &'a [String],
            assignees: &'a [String],
        }

        let frontmatter = Frontmatter {
            title: self.title.as_deref().unwrap_or(""),
            labels: &self.labels,
            assignees: &self.assignees,
        };

        // serde_yaml::to_string is safe for the struct above and won't fail
        let frontmatter_yaml = serde_yaml::to_string(&frontmatter)
            .unwrap_or_else(|_| "title: \"\"\nlabels: []\nassignees: []".to_string());
        let body = self.body.as_deref().unwrap_or("Body");

        format!("---\n{}---\n\n{}\n", frontmatter_yaml, body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    fn create_template(
        name: &str,
        title: Option<&str>,
        body: Option<&str>,
        labels: Vec<&str>,
        assignees: Vec<&str>,
    ) -> IssueTemplate {
        IssueTemplate {
            name: name.to_string(),
            title: title.map(|s| s.to_string()),
            body: body.map(|s| s.to_string()),
            about: None,
            filename: None,
            labels: labels.into_iter().map(|s| s.to_string()).collect(),
            assignees: assignees.into_iter().map(|s| s.to_string()).collect(),
        }
    }

    #[rstest]
    fn test_to_issue_content_with_all_fields() {
        let template = create_template(
            "Bug Report",
            Some("Bug: "),
            Some("Describe the bug here"),
            vec!["bug", "needs-triage"],
            vec!["alice", "bob"],
        );

        let content = template.to_issue_content();
        // serde_yaml uses single quotes for strings and multi-line format for arrays
        assert!(content.contains("title: 'Bug: '"));
        assert!(content.contains("- bug"));
        assert!(content.contains("- needs-triage"));
        assert!(content.contains("- alice"));
        assert!(content.contains("- bob"));
        assert!(content.contains("Describe the bug here"));
    }

    #[rstest]
    fn test_to_issue_content_with_empty_fields() {
        let template = create_template("Empty", None, None, vec![], vec![]);

        let content = template.to_issue_content();
        assert!(content.starts_with("---\n"));
        assert!(content.contains("title:"));
        assert!(content.contains("labels: []"));
        assert!(content.contains("assignees: []"));
        assert!(content.ends_with("---\n\nBody\n"));
    }

    #[rstest]
    fn test_to_issue_content_with_single_label() {
        let template = create_template(
            "Feature",
            Some("Feature: "),
            None,
            vec!["enhancement"],
            vec![],
        );

        let content = template.to_issue_content();
        assert!(content.contains("title: 'Feature: '"));
        assert!(content.contains("- enhancement"));
        assert!(content.contains("assignees: []"));
    }

    #[rstest]
    fn test_to_issue_content_preserves_body_content() {
        let body = "## Steps to reproduce\n\n1. First step\n2. Second step\n\n## Expected behavior";
        let template = create_template("Bug", None, Some(body), vec![], vec![]);

        let content = template.to_issue_content();
        assert!(content.contains(body));
    }
}
