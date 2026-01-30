//! Common utilities shared across issue-agent commands.

use std::io::{self, Write};
use std::path::Path;

use crossterm::style::{Color, ResetColor, SetForegroundColor};
use crossterm::tty::IsTty;
use similar::{ChangeTag, TextDiff};

use crate::infra::git;

// Re-export git::parse_repo for convenience.
// Note: This returns git::Result, callers using Box<dyn Error> can use `?` directly.
pub use git::parse_repo;

/// Get repository from argument or git remote origin.
///
/// If a repo argument is provided, returns it directly.
/// Otherwise, attempts to determine the repository from git remote origin.
pub fn get_repo_from_arg_or_git(repo_arg: &Option<String>) -> anyhow::Result<String> {
    if let Some(repo) = repo_arg {
        return Ok(repo.clone());
    }

    // Get from git remote origin
    let (owner, repo) = git::get_owner_repo().ok_or_else(|| {
        anyhow::anyhow!("Failed to determine current repository. Use -R to specify.")
    })?;

    Ok(format!("{}/{}", owner, repo))
}

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

/// Print success message after fetching issue.
pub fn print_fetch_success(issue_number: u64, title: &str, dir: &Path) {
    eprintln!();
    eprintln!(
        "Done! Issue #{issue_number} has been saved to {}/",
        dir.display()
    );
    eprintln!();
    eprintln!("Title: {title}");
    eprintln!();
    eprintln!("Files:");
    eprintln!(
        "  {}/issue.md          - Issue body (editable)",
        dir.display()
    );
    eprintln!(
        "  {}/metadata.json     - Metadata (editable: title, labels, assignees)",
        dir.display()
    );
    eprintln!(
        "  {}/comments/         - Comments (only your own comments are editable)",
        dir.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    // parse_repo tests are in src/git/repo.rs

    mod get_repo_tests {
        use super::*;

        #[rstest]
        #[case::simple("owner/repo")]
        #[case::real_repo("fohte/armyknife")]
        #[case::with_special_chars("my-org/my_repo.rs")]
        fn test_with_arg_returns_as_is(#[case] repo: &str) {
            let result = get_repo_from_arg_or_git(&Some(repo.to_string())).unwrap();
            assert_eq!(result, repo);
        }
    }

    mod diff_tests {
        use super::*;

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
    }
}
