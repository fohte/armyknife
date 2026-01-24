//! GitHub-related git operations.

use git2::Repository;
use lazy_regex::regex_captures;

use super::error::{GitError, Result};
use super::repo::{open_repo, origin_url};

/// Parse owner and repo from a GitHub URL.
/// Supports both SSH (git@github.com:owner/repo.git) and HTTPS formats.
pub fn parse_github_url(url: &str) -> Result<(String, String)> {
    let (_, owner, repo) = regex_captures!(r"(?:github\.com[:/])([^/]+)/([^/]+?)(?:\.git)?$", url)
        .ok_or_else(|| GitError::InvalidGitHubUrl(url.to_string()))?;
    Ok((owner.to_string(), repo.to_string()))
}

/// Get owner and repo from the origin remote.
pub fn github_owner_and_repo(repo: &Repository) -> Result<(String, String)> {
    let url = origin_url(repo)?;
    parse_github_url(&url)
}

/// Get owner and repo from git remote URL (using current repository).
pub fn get_owner_repo() -> Option<(String, String)> {
    let repo = open_repo().ok()?;
    let remote = repo.find_remote("origin").ok()?;
    let url = remote.url()?;
    parse_github_url(url).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::https("https://github.com/owner/repo.git", "owner", "repo")]
    #[case::https_no_git("https://github.com/owner/repo", "owner", "repo")]
    #[case::ssh("git@github.com:owner/repo.git", "owner", "repo")]
    #[case::ssh_no_git("git@github.com:owner/repo", "owner", "repo")]
    fn test_parse_github_url(
        #[case] url: &str,
        #[case] expected_owner: &str,
        #[case] expected_repo: &str,
    ) {
        let (owner, repo) = parse_github_url(url).unwrap();
        assert_eq!(owner, expected_owner);
        assert_eq!(repo, expected_repo);
    }

    #[rstest]
    #[case::not_github("https://gitlab.com/owner/repo.git")]
    #[case::invalid("not-a-url")]
    fn test_parse_github_url_invalid(#[case] url: &str) {
        assert!(parse_github_url(url).is_err());
    }
}
