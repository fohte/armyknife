//! Editable frontmatter fields shared between new-issue creation and existing-issue editing.
//!
//! Both `NewIssueFrontmatter` (create path) and `IssueFrontmatter` (edit path) embed this
//! via `#[serde(flatten)]`, so adding a field here automatically propagates to both paths.

use serde::{Deserialize, Serialize};

/// User-editable issue frontmatter fields.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EditableIssueFields {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub assignees: Vec<String>,
    #[serde(default)]
    pub milestone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_issue: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sub_issues: Vec<String>,
}

impl EditableIssueFields {
    /// Field names accepted in **new-issue** frontmatter, in the camelCase
    /// they appear in YAML. The edit path reads its frontmatter via plain
    /// `serde_yaml::from_str` (no allow-list check), so this list only
    /// constrains `NewIssue::parse`.
    ///
    /// `milestone` is intentionally absent: the create API path does not
    /// reconcile milestones today, so accepting it on new-issue frontmatter
    /// would silently drop the value. Edit-path frontmatter still carries
    /// `milestone` round-tripped from the remote.
    pub const KNOWN_KEYS_FOR_NEW_ISSUE: &'static [&'static str] =
        &["title", "labels", "assignees", "parentIssue", "subIssues"];
}
