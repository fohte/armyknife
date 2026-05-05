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

    /// Returns the GitHub GraphQL spelling (SCREAMING_SNAKE_CASE) used in markdown headers.
    pub fn as_graphql_str(&self) -> &'static str {
        match self {
            Self::Approved => "APPROVED",
            Self::ChangesRequested => "CHANGES_REQUESTED",
            Self::Commented => "COMMENTED",
            Self::Dismissed => "DISMISSED",
            Self::Pending => "PENDING",
        }
    }

    /// Parse the GraphQL spelling back into a `ReviewState`.
    pub fn from_graphql_str(s: &str) -> Option<Self> {
        match s {
            "APPROVED" => Some(Self::Approved),
            "CHANGES_REQUESTED" => Some(Self::ChangesRequested),
            "COMMENTED" => Some(Self::Commented),
            "DISMISSED" => Some(Self::Dismissed),
            "PENDING" => Some(Self::Pending),
            _ => None,
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
    #[case::approved(ReviewState::Approved, "APPROVED")]
    #[case::changes_requested(ReviewState::ChangesRequested, "CHANGES_REQUESTED")]
    #[case::commented(ReviewState::Commented, "COMMENTED")]
    #[case::dismissed(ReviewState::Dismissed, "DISMISSED")]
    #[case::pending(ReviewState::Pending, "PENDING")]
    fn test_review_state_as_graphql_str(#[case] state: ReviewState, #[case] expected: &str) {
        assert_eq!(state.as_graphql_str(), expected);
    }

    #[rstest]
    #[case::approved("APPROVED", Some(ReviewState::Approved))]
    #[case::changes_requested("CHANGES_REQUESTED", Some(ReviewState::ChangesRequested))]
    #[case::commented("COMMENTED", Some(ReviewState::Commented))]
    #[case::dismissed("DISMISSED", Some(ReviewState::Dismissed))]
    #[case::pending("PENDING", Some(ReviewState::Pending))]
    #[case::unknown("WAT", None)]
    fn test_review_state_from_graphql_str(
        #[case] input: &str,
        #[case] expected: Option<ReviewState>,
    ) {
        assert_eq!(ReviewState::from_graphql_str(input), expected);
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
