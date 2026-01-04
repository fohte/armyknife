use serde::Deserialize;

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

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplyTo {
    pub database_id: i64,
}

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
        let review_ids: Vec<i64> = self.reviews.iter().map(|r| r.database_id).collect();
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

    /// Get replies to the root comment
    pub fn replies(&self) -> Vec<&Comment> {
        let root_id = self.root_comment().map(|c| c.database_id);
        self.comments
            .nodes
            .iter()
            .filter(|c| {
                c.reply_to
                    .as_ref()
                    .is_some_and(|r| Some(r.database_id) == root_id)
            })
            .collect()
    }

    /// Count unresolved threads in a list
    pub fn count_unresolved(threads: &[&ReviewThread]) -> usize {
        threads.iter().filter(|t| !t.is_resolved).count()
    }
}
