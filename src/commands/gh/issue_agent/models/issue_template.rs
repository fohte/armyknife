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
        let title = self.title.as_deref().unwrap_or("");
        let body = self.body.as_deref().unwrap_or("Body");

        let labels_yaml = if self.labels.is_empty() {
            "[]".to_string()
        } else {
            format!("[{}]", self.labels.join(", "))
        };

        let assignees_yaml = if self.assignees.is_empty() {
            "[]".to_string()
        } else {
            format!("[{}]", self.assignees.join(", "))
        };

        format!(
            "---\ntitle: \"{}\"\nlabels: {}\nassignees: {}\n---\n\n{}\n",
            title, labels_yaml, assignees_yaml, body
        )
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
        assert_eq!(
            content,
            "---\n\
             title: \"Bug: \"\n\
             labels: [bug, needs-triage]\n\
             assignees: [alice, bob]\n\
             ---\n\n\
             Describe the bug here\n"
        );
    }

    #[rstest]
    fn test_to_issue_content_with_empty_fields() {
        let template = create_template("Empty", None, None, vec![], vec![]);

        let content = template.to_issue_content();
        assert_eq!(
            content,
            "---\n\
             title: \"\"\n\
             labels: []\n\
             assignees: []\n\
             ---\n\n\
             Body\n"
        );
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
        assert!(content.contains("title: \"Feature: \""));
        assert!(content.contains("labels: [enhancement]"));
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
