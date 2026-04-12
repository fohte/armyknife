//! Devin AI review detector.

use crate::commands::ai::review::detector::{CompletionDetection, ReviewDetector, StartDetection};

pub struct DevinDetector;

impl ReviewDetector for DevinDetector {
    fn bot_login(&self) -> &'static str {
        "devin-ai-integration"
    }

    fn unable_marker(&self) -> &'static str {
        "Devin is unable to"
    }

    fn start_method(&self) -> StartDetection {
        StartDetection::CheckRun {
            name: "devin-review",
        }
    }

    fn completion_method(&self) -> CompletionDetection {
        CompletionDetection::ReviewOrCheckRun {
            check_name: "devin-review",
        }
    }
}
