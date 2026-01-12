use super::comment::Author;
use serde::Deserialize;

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

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::approved(ReviewState::Approved, "approved")]
    #[case::changes_requested(ReviewState::ChangesRequested, "changes_requested")]
    #[case::commented(ReviewState::Commented, "commented")]
    #[case::dismissed(ReviewState::Dismissed, "dismissed")]
    #[case::pending(ReviewState::Pending, "pending")]
    fn test_review_state_as_str(#[case] state: ReviewState, #[case] expected: &str) {
        assert_eq!(state.as_str(), expected);
    }

    #[rstest]
    fn test_review_state_deserialize() {
        let approved: ReviewState = serde_json::from_str(r#""APPROVED""#).unwrap();
        assert_eq!(approved, ReviewState::Approved);

        let changes: ReviewState = serde_json::from_str(r#""CHANGES_REQUESTED""#).unwrap();
        assert_eq!(changes, ReviewState::ChangesRequested);

        let commented: ReviewState = serde_json::from_str(r#""COMMENTED""#).unwrap();
        assert_eq!(commented, ReviewState::Commented);
    }

    #[rstest]
    fn test_review_deserialize() {
        let json = r#"{
            "databaseId": 123,
            "author": {"login": "reviewer"},
            "body": "LGTM",
            "state": "APPROVED",
            "createdAt": "2024-01-01T00:00:00Z"
        }"#;
        let review: Review = serde_json::from_str(json).unwrap();
        assert_eq!(review.database_id, 123);
        assert_eq!(review.author.unwrap().login, "reviewer");
        assert_eq!(review.body, "LGTM");
        assert_eq!(review.state, ReviewState::Approved);
    }
}
