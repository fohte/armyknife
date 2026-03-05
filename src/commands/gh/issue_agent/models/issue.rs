use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::author::{Author, WithAuthor};

/// Reference to another issue, used for sub-issue relationships.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubIssueRef {
    /// Issue internal ID (used by Sub-issues API)
    pub id: u64,
    /// Issue number
    pub number: i64,
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
}

impl SubIssueRef {
    /// Format as "owner/repo#number"
    pub fn to_ref_string(&self) -> String {
        format!("{}/{}#{}", self.owner, self.repo, self.number)
    }
}

/// Represents a GitHub Issue fetched from the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub number: i64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub labels: Vec<Label>,
    pub assignees: Vec<Author>,
    pub milestone: Option<Milestone>,
    pub author: Option<Author>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Timestamp when the issue body was last edited (from GraphQL `lastEditedAt`).
    /// None if the issue has never been edited since creation.
    /// Note: GitHub API only provides a single `lastEditedAt` for body edits.
    /// Title edits are detected via `updatedAt` instead.
    #[serde(default)]
    pub last_edited_at: Option<DateTime<Utc>>,
    /// Reference to the parent issue (format: "owner/repo#number"), if this is a sub-issue.
    #[serde(default)]
    pub parent_issue: Option<SubIssueRef>,
    /// List of sub-issues (format: "owner/repo#number").
    #[serde(default)]
    pub sub_issues: Vec<SubIssueRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Label {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Milestone {
    pub title: String,
}

impl WithAuthor for Issue {
    fn author(&self) -> Option<&Author> {
        self.author.as_ref()
    }
}

impl From<octocrab::models::issues::Issue> for Issue {
    fn from(issue: octocrab::models::issues::Issue) -> Self {
        let state = match issue.state {
            octocrab::models::IssueState::Open => "OPEN".to_string(),
            octocrab::models::IssueState::Closed => "CLOSED".to_string(),
            // IssueState is #[non_exhaustive], so handle future variants
            _ => format!("{:?}", issue.state).to_uppercase(),
        };

        Self {
            number: issue.number as i64,
            title: issue.title,
            body: issue.body,
            state,
            labels: issue
                .labels
                .into_iter()
                .map(|l| Label { name: l.name })
                .collect(),
            assignees: issue
                .assignees
                .into_iter()
                .map(|a| Author { login: a.login })
                .collect(),
            milestone: issue.milestone.map(|m| Milestone { title: m.title }),
            author: Some(Author {
                login: issue.user.login,
            }),
            created_at: issue.created_at,
            updated_at: issue.updated_at,
            // REST API doesn't provide this field, it needs to be populated via GraphQL
            last_edited_at: None,
            parent_issue: None,
            sub_issues: vec![],
        }
    }
}
