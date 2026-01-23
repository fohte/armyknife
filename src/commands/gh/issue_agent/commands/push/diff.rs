//! Diff display utilities for push command.

// Re-export from common module for backward compatibility
pub(super) use super::super::common::print_diff;

#[cfg(test)]
use similar::{ChangeTag, TextDiff};

/// Format diff as a string (for testing).
#[cfg(test)]
fn format_diff(old: &str, new: &str) -> String {
    let diff = TextDiff::from_lines(old, new);
    let mut result = String::new();

    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => "-",
            ChangeTag::Insert => "+",
            ChangeTag::Equal => " ",
        };
        result.push_str(sign);
        result.push_str(&change.to_string());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::no_changes("a\n", "a\n", " a\n")]
    #[case::add_line("a\n", "a\nb\n", " a\n+b\n")]
    #[case::delete_line("a\nb\n", "a\n", " a\n-b\n")]
    #[case::modify("old\n", "new\n", "-old\n+new\n")]
    #[case::modify_middle("a\nold\nc\n", "a\nnew\nc\n", " a\n-old\n+new\n c\n")]
    #[case::empty_both("", "", "")]
    fn test_format_diff(#[case] old: &str, #[case] new: &str, #[case] expected: &str) {
        let diff = format_diff(old, new);
        assert_eq!(diff, expected);
    }
}
