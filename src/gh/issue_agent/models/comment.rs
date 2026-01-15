use serde::{Deserialize, Serialize};

use super::issue::Author;

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
    pub created_at: String,
    pub body: String,
}

#[allow(dead_code)]
impl Comment {
    pub fn author_login(&self) -> &str {
        self.author
            .as_ref()
            .map(|a| a.login.as_str())
            .unwrap_or("unknown")
    }
}
