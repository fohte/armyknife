use serde::{Deserialize, Serialize};

use super::author::WithAuthor;
use super::issue::Issue;

/// Metadata stored locally in metadata.json for an issue.
/// This is a flattened representation suitable for local storage.
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

impl IssueMetadata {
    /// Create metadata from an Issue, flattening nested structures.
    pub fn from_issue(issue: &Issue) -> Self {
        Self {
            number: issue.number,
            title: issue.title.clone(),
            state: issue.state.clone(),
            labels: issue.labels.iter().map(|l| l.name.clone()).collect(),
            assignees: issue.assignees.iter().map(|a| a.login.clone()).collect(),
            milestone: issue.milestone.as_ref().map(|m| m.title.clone()),
            author: issue.author_login().to_string(),
            created_at: issue.created_at.to_rfc3339(),
            updated_at: issue.updated_at.to_rfc3339(),
        }
    }
}
