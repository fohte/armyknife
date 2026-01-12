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

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::with_author(Some(Author { login: "user1".to_string() }), "user1")]
    #[case::without_author(None, "unknown")]
    fn test_author_login(#[case] author: Option<Author>, #[case] expected: &str) {
        let comment = Comment {
            database_id: 1,
            author,
            body: String::new(),
            created_at: String::new(),
            path: None,
            line: None,
            original_line: None,
            diff_hunk: None,
            reply_to: None,
            pull_request_review: None,
        };
        assert_eq!(comment.author_login(), expected);
    }

    #[rstest]
    #[case::line_only(Some(10), None, Some(10))]
    #[case::original_line_only(None, Some(20), Some(20))]
    #[case::both(Some(10), Some(20), Some(10))]
    #[case::neither(None, None, None)]
    fn test_effective_line(
        #[case] line: Option<i64>,
        #[case] original_line: Option<i64>,
        #[case] expected: Option<i64>,
    ) {
        let comment = Comment {
            database_id: 1,
            author: None,
            body: String::new(),
            created_at: String::new(),
            path: None,
            line,
            original_line,
            diff_hunk: None,
            reply_to: None,
            pull_request_review: None,
        };
        assert_eq!(comment.effective_line(), expected);
    }
}
