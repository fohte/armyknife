use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    #[allow(dead_code)] // Used in tests
    pub database_id: i64,
    pub author: Option<Author>,
    pub body: String,
    pub created_at: String,
    pub path: Option<String>,
    pub line: Option<i64>,
    pub original_line: Option<i64>,
    pub diff_hunk: Option<String>,
    pub reply_to: Option<ReplyTo>,
    pub pull_request_review: Option<PullRequestReview>,
}

impl Comment {
    pub fn author_login(&self) -> &str {
        self.author
            .as_ref()
            .map(|a| a.login.as_str())
            .unwrap_or("unknown")
    }

    pub fn effective_line(&self) -> Option<i64> {
        self.line.or(self.original_line)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Author {
    pub login: String,
}

/// Marker struct indicating a comment is a reply.
/// The actual databaseId is ignored since we only check for existence.
#[derive(Debug, Clone, Deserialize)]
pub struct ReplyTo {}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestReview {
    pub database_id: i64,
}
