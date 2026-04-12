//! Gemini Code Assist review detector.

use crate::commands::ai::review::detector::{CompletionDetection, ReviewDetector, StartDetection};

pub struct GeminiDetector;

impl ReviewDetector for GeminiDetector {
    fn bot_login(&self) -> &'static str {
        "gemini-code-assist"
    }

    fn review_command(&self) -> Option<&'static str> {
        Some("/gemini review")
    }

    fn unable_marker(&self) -> &'static str {
        "Gemini is unable to"
    }

    fn start_method(&self) -> StartDetection {
        StartDetection::BodyReaction { emoji: "EYES" }
    }

    fn completion_method(&self) -> CompletionDetection {
        CompletionDetection::ReviewSubmitted
    }
}
