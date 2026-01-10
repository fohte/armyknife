mod full;
mod summary;

pub use full::{print_full, print_review_details};
pub use summary::print_summary;

use super::models::{Review, ReviewState};
use regex::Regex;
use std::sync::LazyLock;

static DETAILS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)<details[^>]*>\s*<summary[^>]*>(.*?)</summary>.*?</details>").unwrap()
});

pub struct FormatOptions {
    pub open_details: bool,
}

fn collapse_details(text: &str) -> String {
    DETAILS_RE
        .replace_all(text, |caps: &regex::Captures| format!("[▶ {}]", &caps[1]))
        .to_string()
}

pub(super) fn process_body(body: &str, options: &FormatOptions) -> String {
    if options.open_details {
        body.to_string()
    } else {
        collapse_details(body)
    }
}

pub(super) fn truncate_text(text: &str, max_length: usize) -> String {
    let first_line = text.lines().next().unwrap_or("").trim_start();
    let char_count = first_line.chars().count();
    if char_count > max_length {
        let truncated: String = first_line.chars().take(max_length).collect();
        format!("{truncated}...")
    } else {
        first_line.to_string()
    }
}

pub(super) fn format_datetime(iso_date: &str) -> String {
    iso_date
        .replace('T', " ")
        .replace('Z', "")
        .chars()
        .take(16)
        .collect()
}

pub(super) fn state_indicator(state: ReviewState) -> &'static str {
    match state {
        ReviewState::Approved => "[approved]",
        ReviewState::ChangesRequested => "[changes requested]",
        ReviewState::Commented => "[commented]",
        ReviewState::Dismissed => "[dismissed]",
        ReviewState::Pending => "[pending]",
    }
}

pub(super) fn author_login(review: &Review) -> &str {
    review
        .author
        .as_ref()
        .map(|a| a.login.as_str())
        .unwrap_or("unknown")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collapse_details() {
        let input = "Before <details><summary>Click me</summary>Hidden content</details> After";
        let expected = "Before [▶ Click me] After";
        assert_eq!(collapse_details(input), expected);
    }

    #[test]
    fn test_collapse_details_multiline() {
        let input =
            "Text\n<details>\n<summary>Summary</summary>\nLots of\nhidden\nstuff\n</details>\nMore";
        let result = collapse_details(input);
        assert!(result.contains("[▶ Summary]"));
        assert!(!result.contains("hidden"));
    }

    #[test]
    fn test_truncate_text() {
        assert_eq!(truncate_text("Hello World", 20), "Hello World");
        assert_eq!(truncate_text("Hello World", 5), "Hello...");
        assert_eq!(truncate_text("  Leading spaces", 10), "Leading sp...");
    }

    #[test]
    fn test_truncate_text_utf8() {
        assert_eq!(truncate_text("日本語テスト", 3), "日本語...");
        assert_eq!(truncate_text("こんにちは世界", 5), "こんにちは...");
        assert_eq!(truncate_text("Hello 世界", 7), "Hello 世...");
        assert_eq!(truncate_text("日本語", 10), "日本語");
    }

    #[test]
    fn test_format_datetime() {
        assert_eq!(format_datetime("2024-01-15T10:30:00Z"), "2024-01-15 10:30");
        assert_eq!(
            format_datetime("2024-12-31T23:59:59.123Z"),
            "2024-12-31 23:59"
        );
    }
}
