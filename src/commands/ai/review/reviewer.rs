//! Reviewer definitions.

use clap::ValueEnum;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::detectors::{AnyDetector, DevinDetector, GeminiDetector};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize, JsonSchema, Hash,
)]
#[serde(rename_all = "lowercase")]
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

/// Built-in default reviewer set when no config or CLI override applies.
pub fn builtin_default_reviewers() -> Vec<Reviewer> {
    vec![Reviewer::Gemini, Reviewer::Devin]
}

/// Resolve which reviewers to use, applying the precedence
/// CLI override > repo config > org config > built-in default.
pub fn resolve_reviewers_with_default(
    cli: Option<&[Reviewer]>,
    config: &crate::shared::config::Config,
    owner: &str,
    repo: &str,
) -> Vec<Reviewer> {
    if let Some(reviewers) = cli {
        return reviewers.to_vec();
    }
    if let Some(reviewers) = config.resolve_reviewers(owner, repo) {
        return reviewers;
    }
    builtin_default_reviewers()
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
