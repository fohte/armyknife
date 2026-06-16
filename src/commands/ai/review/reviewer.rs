//! Reviewer definitions.

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use super::detectors::{AnyDetector, CodeRabbitDetector, DevinDetector};

#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Reviewer {
    /// Devin AI
    Devin,
    /// CodeRabbit
    CodeRabbit,
}

impl Reviewer {
    /// Get the detector implementation for this reviewer.
    pub fn detector(&self) -> AnyDetector {
        match self {
            Self::Devin => AnyDetector::Devin(DevinDetector),
            Self::CodeRabbit => AnyDetector::CodeRabbit(CodeRabbitDetector),
        }
    }
}

/// Built-in default reviewer set when no config or CLI override applies.
pub fn builtin_default_reviewers() -> Vec<Reviewer> {
    vec![Reviewer::Devin]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::ai::review::detector::ReviewDetector;
    use rstest::rstest;

    #[rstest]
    // GitHub GraphQL API returns "devin-ai-integration" without "[bot]" suffix
    #[case::devin(Reviewer::Devin, "devin-ai-integration")]
    // GitHub GraphQL API returns "coderabbitai" without "[bot]" suffix
    #[case::coderabbit(Reviewer::CodeRabbit, "coderabbitai")]
    fn bot_login_matches_github_api_response(#[case] reviewer: Reviewer, #[case] expected: &str) {
        assert_eq!(reviewer.detector().bot_login(), expected);
    }
}
