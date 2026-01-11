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
