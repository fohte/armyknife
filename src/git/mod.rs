//! Git operations using git2 (libgit2).
//!
//! This module provides a unified interface for git operations without
//! spawning external git processes.

use git2::Repository;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GitError {
    #[error("Not in a git repository")]
    NotInRepo,

    #[error("No remote 'origin' found")]
    NoOriginRemote,

    #[error("Could not parse GitHub URL: {0}")]
    InvalidGitHubUrl(String),

    #[error("Git error: {0}")]
    Git2(#[from] git2::Error),
}

pub type Result<T> = std::result::Result<T, GitError>;

/// Open a git repository from the current directory or any parent.
pub fn open_repo() -> Result<Repository> {
    Repository::open_from_env().map_err(|_| GitError::NotInRepo)
}

/// Open a git repository from a specific path.
#[cfg(test)]
pub fn open_repo_at(path: &std::path::Path) -> Result<Repository> {
    use git2::RepositoryOpenFlags;
    Repository::open_ext(
        path,
        RepositoryOpenFlags::empty(),
        std::iter::empty::<&std::path::Path>(),
    )
    .map_err(|_| GitError::NotInRepo)
}

/// Get the current branch name.
/// Returns "HEAD" if in detached HEAD state.
pub fn current_branch(repo: &Repository) -> Result<String> {
    let head = repo.head()?;
    Ok(head.shorthand().unwrap_or("HEAD").to_string())
}

/// Get the remote URL for "origin".
pub fn origin_url(repo: &Repository) -> Result<String> {
    let remote = repo
        .find_remote("origin")
        .map_err(|_| GitError::NoOriginRemote)?;
    remote
        .url()
        .map(str::to_string)
        .ok_or(GitError::NoOriginRemote)
}

/// Parse owner and repo from a GitHub URL.
/// Supports both SSH (git@github.com:owner/repo.git) and HTTPS formats.
pub fn parse_github_url(url: &str) -> Result<(String, String)> {
    use regex::Regex;
    use std::sync::LazyLock;

    static GITHUB_URL_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?:github\.com[:/])([^/]+)/([^/]+?)(?:\.git)?$").unwrap());

    if let Some(captures) = GITHUB_URL_RE.captures(url) {
        let owner = captures.get(1).unwrap().as_str().to_string();
        let repo = captures.get(2).unwrap().as_str().to_string();
        Ok((owner, repo))
    } else {
        Err(GitError::InvalidGitHubUrl(url.to_string()))
    }
}

/// Get owner and repo from the origin remote.
pub fn github_owner_and_repo(repo: &Repository) -> Result<(String, String)> {
    let url = origin_url(repo)?;
    parse_github_url(&url)
}

/// Test utilities for creating temporary git repositories.
#[cfg(test)]
pub mod test_utils {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// A temporary git repository for testing.
    pub struct TempRepo {
        pub dir: TempDir,
    }

    impl TempRepo {
        /// Create a new temporary git repository with a GitHub-style origin remote.
        pub fn new(owner: &str, repo_name: &str, branch: &str) -> Self {
            let dir = TempDir::new().expect("create temp dir");
            let repo = Repository::init(dir.path()).expect("init repo");

            // Create initial commit so HEAD exists
            {
                let sig = git2::Signature::now("Test", "test@example.com").unwrap();
                let tree_id = repo.index().unwrap().write_tree().unwrap();
                let tree = repo.find_tree(tree_id).unwrap();
                repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
                    .expect("create initial commit");
            }

            // Rename default branch if needed
            {
                let head = repo.head().expect("get head");
                let current_branch = head.shorthand().unwrap_or("master").to_string();
                drop(head); // Release borrow before renaming
                if current_branch != branch {
                    let mut branch_ref = repo
                        .find_branch(&current_branch, git2::BranchType::Local)
                        .expect("find branch");
                    branch_ref.rename(branch, true).expect("rename branch");
                }
            }

            // Set up origin remote with GitHub URL
            let url = format!("https://github.com/{owner}/{repo_name}.git");
            repo.remote("origin", &url).expect("set origin");

            Self { dir }
        }

        /// Get the path to the repository.
        pub fn path(&self) -> PathBuf {
            self.dir.path().to_path_buf()
        }

        /// Open the repository.
        pub fn open(&self) -> Repository {
            open_repo_at(self.dir.path()).expect("open temp repo")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use test_utils::TempRepo;

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

    #[test]
    fn test_temp_repo_creates_valid_repo() {
        let temp = TempRepo::new("test-owner", "test-repo", "main");
        let repo = temp.open();

        let branch = current_branch(&repo).unwrap();
        assert_eq!(branch, "main");

        let (owner, repo_name) = github_owner_and_repo(&repo).unwrap();
        assert_eq!(owner, "test-owner");
        assert_eq!(repo_name, "test-repo");
    }
}
