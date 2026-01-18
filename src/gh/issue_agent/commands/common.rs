//! Common utilities shared across issue-agent commands.

use std::path::Path;

use crate::git::get_owner_repo;

/// Get repository from argument or git remote origin.
///
/// If a repo argument is provided, returns it directly.
/// Otherwise, attempts to determine the repository from git remote origin.
pub fn get_repo_from_arg_or_git(
    repo_arg: &Option<String>,
) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(repo) = repo_arg {
        return Ok(repo.clone());
    }

    // Get from git remote origin
    let (owner, repo) =
        get_owner_repo().ok_or("Failed to determine current repository. Use -R to specify.")?;

    Ok(format!("{}/{}", owner, repo))
}

/// Parse "owner/repo" into (owner, repo) tuple.
pub fn parse_repo(repo: &str) -> Result<(String, String), Box<dyn std::error::Error>> {
    if let Some((owner, repo_name)) = repo.split_once('/') {
        if owner.is_empty() || repo_name.is_empty() {
            return Err(format!("Invalid repository format: {repo}. Expected owner/repo").into());
        }
        Ok((owner.to_string(), repo_name.to_string()))
    } else {
        Err(format!("Invalid repository format: {repo}. Expected owner/repo").into())
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

    mod parse_repo_tests {
        use super::*;

        #[rstest]
        #[case::valid("owner/repo", ("owner", "repo"))]
        #[case::with_dashes("my-org/my-repo", ("my-org", "my-repo"))]
        #[case::with_numbers("org123/repo456", ("org123", "repo456"))]
        #[case::with_dots("org.name/repo.name", ("org.name", "repo.name"))]
        fn test_valid(#[case] input: &str, #[case] expected: (&str, &str)) {
            let result = parse_repo(input).unwrap();
            assert_eq!(result, (expected.0.to_string(), expected.1.to_string()));
        }

        #[rstest]
        #[case::no_slash("ownerrepo")]
        #[case::empty("")]
        #[case::only_slash("/")]
        #[case::empty_owner("/repo")]
        #[case::empty_repo("owner/")]
        fn test_invalid(#[case] input: &str) {
            let result = parse_repo(input);
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("Invalid repository format")
            );
        }

        #[test]
        fn test_multiple_slashes_takes_first() {
            // split_once splits at first occurrence, so "a/b/c" -> ("a", "b/c")
            let result = parse_repo("org/repo/extra").unwrap();
            assert_eq!(result, ("org".to_string(), "repo/extra".to_string()));
        }
    }

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
