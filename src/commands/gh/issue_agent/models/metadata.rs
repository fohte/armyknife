use serde::{Deserialize, Serialize};

use super::author::WithAuthor;
use super::issue::Issue;

/// Read-only metadata fields that should not be edited by users.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReadonlyMetadata {
    pub number: i64,
    pub state: String,
    pub author: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Issue frontmatter stored in issue.md.
/// Editable fields are at the top level, read-only fields are nested under `readonly`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IssueFrontmatter {
    pub title: String,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub assignees: Vec<String>,
    #[serde(default)]
    pub milestone: Option<String>,
    pub readonly: ReadonlyMetadata,
}

impl IssueFrontmatter {
    /// Create frontmatter from an Issue.
    pub fn from_issue(issue: &Issue) -> Self {
        Self {
            title: issue.title.clone(),
            labels: issue.labels.iter().map(|l| l.name.clone()).collect(),
            assignees: issue.assignees.iter().map(|a| a.login.clone()).collect(),
            milestone: issue.milestone.as_ref().map(|m| m.title.clone()),
            readonly: ReadonlyMetadata {
                number: issue.number,
                state: issue.state.clone(),
                author: issue.author_login().to_string(),
                created_at: issue.created_at.to_rfc3339(),
                updated_at: issue.updated_at.to_rfc3339(),
            },
        }
    }
}

/// Legacy metadata stored locally in metadata.json for an issue.
/// This is kept for backward compatibility during migration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueMetadata {
    pub number: i64,
    pub title: String,
    pub state: String,
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
    pub milestone: Option<String>,
    pub author: String,
    pub created_at: String,
    pub updated_at: String,
}

impl From<IssueFrontmatter> for IssueMetadata {
    fn from(fm: IssueFrontmatter) -> Self {
        Self {
            number: fm.readonly.number,
            title: fm.title,
            state: fm.readonly.state,
            labels: fm.labels,
            assignees: fm.assignees,
            milestone: fm.milestone,
            author: fm.readonly.author,
            created_at: fm.readonly.created_at,
            updated_at: fm.readonly.updated_at,
        }
    }
}
