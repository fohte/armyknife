//! Reviewer definitions.

use clap::ValueEnum;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Reviewer {
    /// Gemini Code Assist
    Gemini,
}

impl Reviewer {
    /// Get the GitHub login name for the reviewer bot
    pub fn bot_login(&self) -> &'static str {
        match self {
            Self::Gemini => "gemini-code-assist",
        }
    }

    /// Get the command to trigger a review
    pub fn review_command(&self) -> &'static str {
        match self {
            Self::Gemini => "/gemini review",
        }
    }

    /// Get the marker text that indicates the reviewer is unable to review
    pub fn unable_marker(&self) -> &'static str {
        match self {
            Self::Gemini => "Gemini is unable to",
        }
    }
}
