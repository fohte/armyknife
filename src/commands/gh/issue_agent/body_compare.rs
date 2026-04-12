//! Shared body comparison helper for issue-agent change detection.
//!
//! Both the push-side change detection (`commands/push/detect.rs`) and the
//! diff-side change detection (`storage/issue_storage_diff.rs`) need to decide
//! whether a local body differs from the remote body. The GitHub API may
//! return bodies with trailing `\n`, `\n\n`, or CRLF line endings that are
//! stripped on read, so a raw comparison would surface every no-op pull as a
//! body change. Centralizing the normalization here keeps the two call sites
//! in sync.

/// Normalize body text for comparison.
///
/// Collapses CRLF to LF and strips trailing newlines, spaces, and tabs.
/// Leading whitespace is intentionally preserved because indentation can be
/// semantically significant in markdown (e.g., indented code blocks).
pub(crate) fn normalize_body_for_compare(body: &str) -> String {
    body.replace("\r\n", "\n")
        .trim_end_matches(['\n', '\r', ' ', '\t'])
        .to_string()
}

/// Returns true if the two bodies are equal after normalization.
pub(crate) fn bodies_equal(a: &str, b: &str) -> bool {
    normalize_body_for_compare(a) == normalize_body_for_compare(b)
}
