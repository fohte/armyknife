use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::author::{Author, WithAuthor};

/// Represents a GitHub Issue fetched from the API.
#[allow(dead_code)]
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
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Label {
    pub name: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Milestone {
    pub title: String,
}

#[allow(dead_code)]
impl WithAuthor for Issue {
    fn author(&self) -> Option<&Author> {
        self.author.as_ref()
    }
}
