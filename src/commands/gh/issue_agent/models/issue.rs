use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::author::{Author, WithAuthor};

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
    /// Timestamp when the issue body was last edited (from GraphQL API).
    /// None if the body has never been edited since creation.
    #[serde(default)]
    pub body_last_edited_at: Option<DateTime<Utc>>,
    /// Timestamp when the issue title was last edited (from GraphQL API).
    /// None if the title has never been edited since creation.
    #[serde(default)]
    pub title_last_edited_at: Option<DateTime<Utc>>,
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
            // REST API doesn't provide these fields, they need to be populated via GraphQL
            body_last_edited_at: None,
            title_last_edited_at: None,
        }
    }
}
