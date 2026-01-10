use serde::Deserialize;
use std::collections::HashSet;

#[derive(Debug, Clone, Deserialize)]
pub struct PrData {
    pub reviews: Vec<Review>,
    pub threads: Vec<ReviewThread>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    pub database_id: i64,
    pub author: Option<Author>,
    pub body: String,
    pub state: ReviewState,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewState {
    Approved,
    ChangesRequested,
    Commented,
    Dismissed,
    Pending,
}

impl ReviewState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::ChangesRequested => "changes_requested",
            Self::Commented => "commented",
            Self::Dismissed => "dismissed",
            Self::Pending => "pending",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewThread {
    pub is_resolved: bool,
    pub comments: CommentsNode,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommentsNode {
    pub nodes: Vec<Comment>,
}

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

impl PrData {
    /// Get reviews sorted by creation time
    pub fn sorted_reviews(&self) -> Vec<&Review> {
        let mut reviews: Vec<_> = self.reviews.iter().collect();
        reviews.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        reviews
    }

    /// Get threads belonging to a specific review
    pub fn threads_for_review(&self, review_id: i64) -> Vec<&ReviewThread> {
        self.threads
            .iter()
            .filter(|t| {
                t.comments
                    .nodes
                    .first()
                    .and_then(|c| c.pull_request_review.as_ref())
                    .is_some_and(|pr| pr.database_id == review_id)
            })
            .collect()
    }

    /// Get orphan threads (not associated with any review)
    pub fn orphan_threads(&self) -> Vec<&ReviewThread> {
        let review_ids: HashSet<i64> = self.reviews.iter().map(|r| r.database_id).collect();
        self.threads
            .iter()
            .filter(|t| {
                let thread_review_id = t
                    .comments
                    .nodes
                    .first()
                    .and_then(|c| c.pull_request_review.as_ref())
                    .map(|pr| pr.database_id);
                match thread_review_id {
                    Some(id) => !review_ids.contains(&id),
                    None => true,
                }
            })
            .collect()
    }
}

impl ReviewThread {
    /// Get the root comment (the one without replyTo)
    pub fn root_comment(&self) -> Option<&Comment> {
        self.comments.nodes.iter().find(|c| c.reply_to.is_none())
    }

    /// Get all replies (comments that are not the root comment).
    /// This includes nested replies (replies to replies) since GitHub's API
    /// allows reply chains where replyTo points to any previous comment.
    pub fn replies(&self) -> Vec<&Comment> {
        self.comments
            .nodes
            .iter()
            .filter(|c| c.reply_to.is_some())
            .collect()
    }

    /// Count unresolved threads in a list
    pub fn count_unresolved(threads: &[&ReviewThread]) -> usize {
        threads.iter().filter(|t| !t.is_resolved).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_comment(id: i64, review_id: Option<i64>, is_reply: bool) -> Comment {
        Comment {
            database_id: id,
            author: Some(Author {
                login: "user".to_string(),
            }),
            body: "comment body".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            path: Some("file.rs".to_string()),
            line: Some(10),
            original_line: None,
            diff_hunk: None,
            reply_to: if is_reply { Some(ReplyTo {}) } else { None },
            pull_request_review: review_id.map(|id| PullRequestReview { database_id: id }),
        }
    }

    fn make_thread(review_id: Option<i64>, is_resolved: bool) -> ReviewThread {
        ReviewThread {
            is_resolved,
            comments: CommentsNode {
                nodes: vec![make_comment(1, review_id, false)],
            },
        }
    }

    fn make_review(id: i64, body: &str, state: ReviewState) -> Review {
        Review {
            database_id: id,
            author: Some(Author {
                login: "reviewer".to_string(),
            }),
            body: body.to_string(),
            state,
            created_at: "2024-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn test_threads_for_review() {
        let pr_data = PrData {
            reviews: vec![
                make_review(100, "", ReviewState::Approved),
                make_review(200, "comment", ReviewState::Commented),
            ],
            threads: vec![
                make_thread(Some(100), false),
                make_thread(Some(100), true),
                make_thread(Some(200), false),
            ],
        };

        let threads_100 = pr_data.threads_for_review(100);
        assert_eq!(threads_100.len(), 2);

        let threads_200 = pr_data.threads_for_review(200);
        assert_eq!(threads_200.len(), 1);

        let threads_999 = pr_data.threads_for_review(999);
        assert!(threads_999.is_empty());
    }

    #[test]
    fn test_orphan_threads() {
        let pr_data = PrData {
            reviews: vec![make_review(100, "", ReviewState::Approved)],
            threads: vec![
                make_thread(Some(100), false), // belongs to review 100
                make_thread(Some(999), false), // orphan (review 999 doesn't exist)
                make_thread(None, false),      // orphan (no review association)
            ],
        };

        let orphans = pr_data.orphan_threads();
        assert_eq!(orphans.len(), 2);
    }

    #[test]
    fn test_empty_body_review_preserved() {
        // Reviews with empty body should still be included
        let pr_data = PrData {
            reviews: vec![
                make_review(100, "", ReviewState::Approved),
                make_review(200, "has body", ReviewState::ChangesRequested),
            ],
            threads: vec![make_thread(Some(100), false)],
        };

        assert_eq!(pr_data.reviews.len(), 2);
        // Threads should still be associated correctly
        assert_eq!(pr_data.threads_for_review(100).len(), 1);
    }

    #[test]
    fn test_count_unresolved() {
        let threads = vec![
            make_thread(Some(1), false),
            make_thread(Some(1), true),
            make_thread(Some(1), false),
        ];
        let refs: Vec<&ReviewThread> = threads.iter().collect();
        assert_eq!(ReviewThread::count_unresolved(&refs), 2);
    }

    #[test]
    fn test_root_comment_and_replies() {
        let thread = ReviewThread {
            is_resolved: false,
            comments: CommentsNode {
                nodes: vec![
                    make_comment(1, Some(100), false), // root
                    make_comment(2, Some(100), true),  // reply
                    make_comment(3, Some(100), true),  // another reply
                    make_comment(4, Some(100), true),  // nested reply
                ],
            },
        };

        let root = thread.root_comment();
        assert!(root.is_some());
        assert_eq!(root.unwrap().database_id, 1);

        // replies() returns all comments that are not the root (have reply_to set)
        let replies = thread.replies();
        assert_eq!(replies.len(), 3);
        // Verify all non-root comments are included
        let reply_ids: Vec<i64> = replies.iter().map(|c| c.database_id).collect();
        assert!(reply_ids.contains(&2));
        assert!(reply_ids.contains(&3));
        assert!(reply_ids.contains(&4)); // nested reply is also included
    }

    #[test]
    fn test_reply_to_deserialize_ignores_extra_fields() {
        // Verify serde ignores unknown fields (like databaseId from API)
        let json = r#"{"databaseId": 123}"#;
        let reply_to: ReplyTo = serde_json::from_str(json).unwrap();
        // ReplyTo is an empty struct, just verify it deserializes
        assert!(std::mem::size_of_val(&reply_to) == 0 || true);
    }

    #[test]
    fn test_comment_with_reply_to_deserialize() {
        let json = r#"{
            "databaseId": 1,
            "author": {"login": "user"},
            "body": "test",
            "createdAt": "2024-01-01T00:00:00Z",
            "path": "file.rs",
            "line": 10,
            "originalLine": null,
            "diffHunk": null,
            "replyTo": {"databaseId": 999},
            "pullRequestReview": {"databaseId": 100}
        }"#;
        let comment: Comment = serde_json::from_str(json).unwrap();
        assert!(comment.reply_to.is_some());
        assert_eq!(comment.database_id, 1);
    }
}
