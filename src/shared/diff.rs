//! Unified diff utilities for displaying text differences.

use std::io::{self, Write};

use crossterm::style::{Color, ResetColor, SetForegroundColor};
use crossterm::tty::IsTty;
use similar::{ChangeTag, TextDiff};

/// Print unified diff between old and new text to stdout with colors.
/// Ignores BrokenPipe errors (e.g., when piped to `head`).
pub fn print_diff(old: &str, new: &str) -> anyhow::Result<()> {
    let use_color = io::stdout().is_tty();
    if let Err(e) = write_diff(&mut io::stdout(), old, new, use_color)
        && e.kind() != io::ErrorKind::BrokenPipe
    {
        return Err(e.into());
    }
    Ok(())
}

/// Print unified diff to stderr with colors.
/// Ignores BrokenPipe errors.
pub fn eprint_diff(old: &str, new: &str) {
    let use_color = io::stderr().is_tty();
    let _ = write_diff(&mut io::stderr(), old, new, use_color);
}

/// Write unified diff between old and new text to a writer.
/// If `use_color` is true, deleted lines are red and inserted lines are green.
pub fn write_diff<W: Write>(
    writer: &mut W,
    old: &str,
    new: &str,
    use_color: bool,
) -> io::Result<()> {
    let diff = TextDiff::from_lines(old, new);
    for change in diff.iter_all_changes() {
        let (sign, color) = match change.tag() {
            ChangeTag::Delete => ("-", Some(Color::Red)),
            ChangeTag::Insert => ("+", Some(Color::Green)),
            ChangeTag::Equal => (" ", None),
        };

        if use_color && let Some(c) = color {
            write!(writer, "{}", SetForegroundColor(c))?;
        }

        // change already includes newline, so no newline here
        write!(writer, "{}{}", sign, change)?;

        if use_color && color.is_some() {
            write!(writer, "{}", ResetColor)?;
        }
    }
    Ok(())
}

/// Format unified diff as a string.
/// If `use_color` is true, deleted lines are red and inserted lines are green.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "kept as public API for future use")
)]
pub fn format_diff(old: &str, new: &str, use_color: bool) -> String {
    let mut output = Vec::new();
    // write_diff only fails on I/O errors, Vec<u8> won't fail
    let _ = write_diff(&mut output, old, new, use_color);
    String::from_utf8(output).unwrap_or_default()
}

/// Print a single line with color (for simple -/+ diffs like title).
pub fn print_colored_line(prefix: &str, text: &str, color: Color) {
    let use_color = io::stdout().is_tty();
    if use_color {
        println!(
            "{}{}{}{}",
            SetForegroundColor(color),
            prefix,
            text,
            ResetColor
        );
    } else {
        println!("{}{}", prefix, text);
    }
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
    fn test_write_diff_no_color(#[case] old: &str, #[case] new: &str, #[case] expected: &str) {
        let mut output = Vec::new();
        write_diff(&mut output, old, new, false).unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), expected);
    }

    #[rstest]
    #[case::no_changes("a\n", "a\n", " a\n")]
    #[case::add_line("a\n", "a\nb\n", " a\n+b\n")]
    #[case::modify("old\n", "new\n", "-old\n+new\n")]
    fn test_format_diff_no_color(#[case] old: &str, #[case] new: &str, #[case] expected: &str) {
        let result = format_diff(old, new, false);
        assert_eq!(result, expected);
    }

    #[rstest]
    fn test_format_diff_with_color_includes_ansi_codes() {
        let result = format_diff("old\n", "new\n", true);
        // Should contain ANSI escape sequences
        assert!(result.contains("\x1b["));
    }
}
