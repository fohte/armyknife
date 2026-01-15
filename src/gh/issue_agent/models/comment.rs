use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::author::{Author, WithAuthor};

/// Represents a comment on a GitHub Issue.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    /// GraphQL node ID
    pub id: String,
    /// REST API database ID
    pub database_id: i64,
    pub author: Option<Author>,
    pub created_at: DateTime<Utc>,
    pub body: String,
}

#[allow(dead_code)]
impl WithAuthor for Comment {
    fn author(&self) -> Option<&Author> {
        self.author.as_ref()
    }
}
