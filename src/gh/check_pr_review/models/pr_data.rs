use super::review::Review;
use super::thread::ReviewThread;
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Debug, Clone, Deserialize)]
pub struct PrData {
    pub reviews: Vec<Review>,
    pub threads: Vec<ReviewThread>,
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
