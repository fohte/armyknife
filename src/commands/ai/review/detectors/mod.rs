//! Bot-specific review detector implementations.

mod devin;
mod gemini;

pub use devin::DevinDetector;
pub use gemini::GeminiDetector;

use super::detector::{
    CompletionDetection, DetectionClient, DetectionContext, ReviewDetector, StartDetection,
};
use super::error::Result;
use chrono::{DateTime, Utc};

/// Enum dispatch for ReviewDetector, allowing static dispatch without dyn.
pub enum AnyDetector {
    Gemini(GeminiDetector),
    Devin(DevinDetector),
}

impl ReviewDetector for AnyDetector {
    fn bot_login(&self) -> &'static str {
        match self {
            Self::Gemini(d) => d.bot_login(),
            Self::Devin(d) => d.bot_login(),
        }
    }

    fn review_command(&self) -> Option<&'static str> {
        match self {
            Self::Gemini(d) => d.review_command(),
            Self::Devin(d) => d.review_command(),
        }
    }

    fn unable_marker(&self) -> &'static str {
        match self {
            Self::Gemini(d) => d.unable_marker(),
            Self::Devin(d) => d.unable_marker(),
        }
    }

    fn start_method(&self) -> StartDetection {
        match self {
            Self::Gemini(d) => d.start_method(),
            Self::Devin(d) => d.start_method(),
        }
    }

    fn completion_method(&self) -> CompletionDetection {
        match self {
            Self::Gemini(d) => d.completion_method(),
            Self::Devin(d) => d.completion_method(),
        }
    }

    async fn is_started<C: DetectionClient>(&self, ctx: &DetectionContext<'_, C>) -> Result<bool> {
        match self {
            Self::Gemini(d) => d.is_started(ctx).await,
            Self::Devin(d) => d.is_started(ctx).await,
        }
    }

    async fn is_completed<C: DetectionClient>(
        &self,
        ctx: &DetectionContext<'_, C>,
    ) -> Result<Option<DateTime<Utc>>> {
        match self {
            Self::Gemini(d) => d.is_completed(ctx).await,
            Self::Devin(d) => d.is_completed(ctx).await,
        }
    }

    async fn find_unable_comment<C: DetectionClient>(
        &self,
        ctx: &DetectionContext<'_, C>,
        after: DateTime<Utc>,
    ) -> Result<Option<String>> {
        match self {
            Self::Gemini(d) => d.find_unable_comment(ctx, after).await,
            Self::Devin(d) => d.find_unable_comment(ctx, after).await,
        }
    }

    async fn request_review<C: DetectionClient>(
        &self,
        ctx: &DetectionContext<'_, C>,
    ) -> Result<()> {
        match self {
            Self::Gemini(d) => d.request_review(ctx).await,
            Self::Devin(d) => d.request_review(ctx).await,
        }
    }
}
