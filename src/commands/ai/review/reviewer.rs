//! Reviewer definitions.

use clap::ValueEnum;

use super::detectors::{AnyDetector, DevinDetector, GeminiDetector};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Reviewer {
    /// Gemini Code Assist
    Gemini,
    /// Devin AI
    Devin,
}

impl Reviewer {
    /// Get the detector implementation for this reviewer.
    pub fn detector(&self) -> AnyDetector {
        match self {
            Self::Gemini => AnyDetector::Gemini(GeminiDetector),
            Self::Devin => AnyDetector::Devin(DevinDetector),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::ai::review::detector::ReviewDetector;
    use rstest::rstest;

    #[rstest]
    #[case::gemini(Reviewer::Gemini, "gemini-code-assist")]
    // GitHub GraphQL API returns "devin-ai-integration" without "[bot]" suffix
    #[case::devin(Reviewer::Devin, "devin-ai-integration")]
    fn bot_login_matches_github_api_response(#[case] reviewer: Reviewer, #[case] expected: &str) {
        assert_eq!(reviewer.detector().bot_login(), expected);
    }
}
