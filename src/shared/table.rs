//! Table formatting utilities for terminal output.
//!
//! Provides functions for truncating and padding strings to fit fixed-width columns,
//! with proper Unicode width handling for CJK characters.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Truncates a string to fit within the specified display width.
/// Uses Unicode width for proper handling of wide characters (e.g., CJK).
///
/// # Examples
/// ```
/// use armyknife::shared::table::truncate_to_width;
/// assert_eq!(truncate_to_width("hello world", 5), "hello");
/// assert_eq!(truncate_to_width("日本語", 4), "日本");  // Each CJK char is width 2
/// ```
pub fn truncate_to_width(s: &str, max_width: usize) -> String {
    let mut result = String::new();
    let mut current_width = 0;

    for c in s.chars() {
        let char_width = c.width().unwrap_or(0);
        if current_width + char_width > max_width {
            break;
        }
        result.push(c);
        current_width += char_width;
    }

    result
}

/// Pads or truncates a string to exactly the specified display width.
/// Uses Unicode display width for proper alignment with CJK characters.
///
/// - If the string is shorter than `width`, pads with spaces on the right.
/// - If the string is longer than `width`, truncates and adds "..." (if width >= 3).
/// - If width < 3, truncates without ellipsis.
///
/// # Examples
/// ```
/// use armyknife::shared::table::pad_or_truncate;
/// assert_eq!(pad_or_truncate("hello", 10), "hello     ");
/// assert_eq!(pad_or_truncate("hello world", 8), "hello...");
/// assert_eq!(pad_or_truncate("日本語", 8), "日本語  ");
/// ```
pub fn pad_or_truncate(s: &str, width: usize) -> String {
    let display_width = s.width();

    if display_width <= width {
        // Pad with spaces to reach target width
        let padding = width - display_width;
        format!("{}{}", s, " ".repeat(padding))
    } else if width < 3 {
        // Too short for ellipsis, just truncate
        truncate_to_width(s, width)
    } else {
        // Truncate and add ellipsis
        let truncated = truncate_to_width(s, width - 3);
        let truncated_width = truncated.width();
        // Use saturating_sub to avoid underflow when CJK chars cause width mismatch
        let padding = width.saturating_sub(truncated_width).saturating_sub(3);
        format!("{}...{}", truncated, " ".repeat(padding))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::ascii_short("hello", 10, "hello")]
    #[case::ascii_exact("hello", 5, "hello")]
    #[case::ascii_truncate("hello world", 5, "hello")]
    #[case::empty("", 5, "")]
    #[case::zero_width("hello", 0, "")]
    #[case::cjk_full("日本語", 6, "日本語")]
    #[case::cjk_truncate("日本語", 4, "日本")]
    #[case::cjk_odd_width("日本語", 5, "日本")]
    #[case::mixed("hello日本語", 9, "hello日本")]
    fn test_truncate_to_width(
        #[case] input: &str,
        #[case] max_width: usize,
        #[case] expected: &str,
    ) {
        assert_eq!(truncate_to_width(input, max_width), expected);
    }

    #[rstest]
    #[case::short("hello", 10, "hello     ")]
    #[case::exact("hello", 5, "hello")]
    #[case::truncate("hello world", 8, "hello...")]
    #[case::truncate_short("hello", 4, "h...")]
    #[case::max_len_3("hello", 3, "...")]
    #[case::max_len_2("hello", 2, "he")]
    #[case::max_len_1("hello", 1, "h")]
    #[case::max_len_0("hello", 0, "")]
    #[case::cjk_short("日本語", 10, "日本語    ")]
    #[case::cjk_exact("日本語", 6, "日本語")]
    #[case::cjk_truncate("日本語テスト", 8, "日本... ")]
    fn test_pad_or_truncate(#[case] input: &str, #[case] width: usize, #[case] expected: &str) {
        assert_eq!(pad_or_truncate(input, width), expected);
    }
}
