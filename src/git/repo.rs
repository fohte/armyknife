//! Repository operations.

use git2::{BranchType, FetchOptions, Repository};
use std::path::Path;

use super::error::{GitError, Result};

/// Open a git repository from the current directory or any parent.
pub fn open_repo() -> Result<Repository> {
    Repository::open_from_env().map_err(|_| GitError::NotInRepo)
}

/// Open a git repository from a specific path.
pub fn open_repo_at(path: &Path) -> Result<Repository> {
    use git2::RepositoryOpenFlags;
    Repository::open_ext(
        path,
        RepositoryOpenFlags::empty(),
        std::iter::empty::<&Path>(),
    )
    .map_err(|_| GitError::NotInRepo)
}

/// Get the main worktree root (the first entry in `git worktree list`).
/// This is always the main repository, regardless of which worktree we're in.
/// For bare repositories, this is the bare repo directory.
/// For regular repositories, this is the main working tree root.
pub fn get_repo_root() -> Result<String> {
    let cwd = std::env::current_dir().map_err(|e| GitError::CommandFailed(e.to_string()))?;
    get_repo_root_in(&cwd)
}

/// Get the main worktree root from the specified directory.
pub fn get_repo_root_in(cwd: &Path) -> Result<String> {
    let repo = open_repo_at(cwd)?;

    let path = if repo.is_worktree() {
        // For worktrees, commondir() points to the main repo's .git
        // The main worktree's workdir is the parent of commondir
        let commondir = repo.commondir();
        commondir.parent().ok_or(GitError::NotInRepo)?
    } else {
        // For the main repo, workdir() gives us the working directory
        repo.workdir().ok_or(GitError::NotInRepo)?
    };

    // Normalize path: remove trailing slash for consistency
    let path_str = path.to_string_lossy();
    Ok(path_str.trim_end_matches('/').to_string())
}

/// Get the current branch name.
/// Returns "HEAD" if in detached HEAD state.
pub fn current_branch(repo: &Repository) -> Result<String> {
    let head = repo.head()?;
    Ok(head.shorthand().unwrap_or("HEAD").to_string())
}

/// Get the main branch name (main or master)
pub fn get_main_branch() -> Result<String> {
    let repo = open_repo()?;
    get_main_branch_for_repo(&repo)
}

/// Get the main branch name for a specific repository
pub fn get_main_branch_for_repo(repo: &Repository) -> Result<String> {
    // Check for origin/main first
    if repo.find_branch("origin/main", BranchType::Remote).is_ok() {
        return Ok("main".to_string());
    }

    // Fall back to master
    Ok("master".to_string())
}

/// Fetch from origin with prune to remove stale remote-tracking references
pub fn fetch_with_prune(repo: &Repository) -> Result<()> {
    let mut remote = repo
        .find_remote("origin")
        .map_err(|e| GitError::CommandFailed(format!("Failed to find origin remote: {e}")))?;

    let mut fetch_opts = FetchOptions::new();
    fetch_opts.prune(git2::FetchPrune::On);

    remote
        .fetch(&[] as &[&str], Some(&mut fetch_opts), None)
        .map_err(|e| GitError::CommandFailed(format!("git fetch failed: {e}")))?;

    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_utils::TempRepo;

    #[test]
    fn test_get_main_branch_for_repo_returns_master_when_no_origin_main() {
        // Create a temp repo without origin/main remote branch
        let temp = TempRepo::new("owner", "repo", "master");
        let repo = temp.open();

        let result = get_main_branch_for_repo(&repo).unwrap();
        assert_eq!(result, "master");
    }

    #[test]
    fn test_get_main_branch_for_repo_returns_main_when_origin_main_exists() {
        let temp = TempRepo::new("owner", "repo", "main");
        let repo = temp.open();

        // Create a fake origin/main remote tracking branch
        // We need to create a reference that looks like a remote branch
        let head_commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.reference(
            "refs/remotes/origin/main",
            head_commit.id(),
            true,
            "create fake remote branch for test",
        )
        .unwrap();

        let result = get_main_branch_for_repo(&repo).unwrap();
        assert_eq!(result, "main");
    }

    #[test]
    fn test_get_main_branch_for_repo_prefers_main_over_master() {
        let temp = TempRepo::new("owner", "repo", "master");
        let repo = temp.open();

        // Create both origin/main and origin/master
        let head_commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.reference(
            "refs/remotes/origin/main",
            head_commit.id(),
            true,
            "create fake origin/main",
        )
        .unwrap();
        repo.reference(
            "refs/remotes/origin/master",
            head_commit.id(),
            true,
            "create fake origin/master",
        )
        .unwrap();

        let result = get_main_branch_for_repo(&repo).unwrap();
        assert_eq!(result, "main");
    }
}
