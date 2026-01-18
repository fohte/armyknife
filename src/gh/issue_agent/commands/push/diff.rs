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
pub(super) fn format_diff(old: &str, new: &str) -> String {
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
