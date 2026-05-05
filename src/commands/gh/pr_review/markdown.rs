mod diff_compress;
mod parser;
pub mod serializer;

pub use parser::{MarkdownParser, ParsedThreadsFile};
pub use serializer::MarkdownSerializer;

// Re-exported for use in sibling modules (changeset tests)
#[cfg(test)]
pub use parser::ParsedThread;

/// HTML-comment markers used to delimit machine-readable sections in `threads.md`.
/// Shared by serializer and parser so the two stay in lock-step.
pub(super) mod markers {
    pub const REVIEWS_OPEN: &str = "<!-- reviews -->";
    pub const REVIEWS_CLOSE: &str = "<!-- /reviews -->";
    pub const REVIEW_OPEN_PREFIX: &str = "<!-- review: ";
    pub const REVIEW_CLOSE: &str = "<!-- /review -->";
}
