//! Common utilities shared across issue-agent commands.

use std::path::Path;

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
}
