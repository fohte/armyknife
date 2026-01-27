//! Reviewer definitions.

use clap::ValueEnum;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Reviewer {
    /// Gemini Code Assist
    Gemini,
    /// Devin AI
    Devin,
}

impl Reviewer {
    /// Get the GitHub login name for the reviewer bot
    pub fn bot_login(&self) -> &'static str {
        match self {
            Self::Gemini => "gemini-code-assist",
            Self::Devin => "devin-ai-integration[bot]",
        }
    }

    /// Get the command to trigger a review.
    /// Returns None if the reviewer does not support command-based review requests.
    pub fn review_command(&self) -> Option<&'static str> {
        match self {
            Self::Gemini => Some("/gemini review"),
            Self::Devin => None,
        }
    }

    /// Get the marker text that indicates the reviewer is unable to review
    pub fn unable_marker(&self) -> &'static str {
        match self {
            Self::Gemini => "Gemini is unable to",
            Self::Devin => "Devin is unable to",
        }
    }

    /// Whether this reviewer requires start detection before waiting.
    /// Gemini posts a summary before the review, so we can detect when it starts.
    /// Devin posts the review directly without any start signal.
    pub fn requires_start_detection(&self) -> bool {
        match self {
            Self::Gemini => true,
            Self::Devin => false,
        }
    }
}
