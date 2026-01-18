//! Diff display utilities for push command.

use similar::{ChangeTag, TextDiff};

/// Print unified diff between old and new text.
pub(super) fn print_diff(old: &str, new: &str) {
    let diff = TextDiff::from_lines(old, new);

    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => "-",
            ChangeTag::Insert => "+",
            ChangeTag::Equal => " ",
        };
        // change already includes newline, so use print! instead of println!
        print!("{}{}", sign, change);
    }
}

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
    #[case::no_changes("a\n", "a\n", vec![" a"])]
    #[case::add_line("a\n", "a\nb\n", vec![" a", "+b"])]
    #[case::delete_line("a\nb\n", "a\n", vec![" a", "-b"])]
    #[case::modify("old\n", "new\n", vec!["-old", "+new"])]
    #[case::modify_middle("a\nold\nc\n", "a\nnew\nc\n", vec![" a", "-old", "+new", " c"])]
    #[case::empty_both("", "", vec![])]
    fn test_format_diff(#[case] old: &str, #[case] new: &str, #[case] expected: Vec<&str>) {
        let diff = format_diff(old, new);
        for line in expected {
            assert!(
                diff.contains(line),
                "Expected '{}' in diff:\n{}",
                line,
                diff
            );
        }
    }
}
