//! Repository operations.

use git2::{BranchType, Cred, FetchOptions, RemoteCallbacks, Repository};
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

/// Get the main branch name for a specific repository.
///
/// Returns `Ok("main")` if `origin/main` exists, `Ok("master")` if `origin/master` exists,
/// or `Err` if neither exists (caller should fall back to GitHub API or default).
pub fn get_main_branch_for_repo(repo: &Repository) -> Result<String> {
    // Check for origin/main first
    if repo.find_branch("origin/main", BranchType::Remote).is_ok() {
        return Ok("main".to_string());
    }

    // Check for origin/master
    if repo
        .find_branch("origin/master", BranchType::Remote)
        .is_ok()
    {
        return Ok("master".to_string());
    }

    // Neither exists - caller should fall back to GitHub API
    Err(GitError::NotFound(
        "Neither origin/main nor origin/master found".to_string(),
    ))
}

/// Fetch from origin with prune to remove stale remote-tracking references
pub fn fetch_with_prune(repo: &Repository) -> Result<()> {
    let mut remote = repo
        .find_remote("origin")
        .map_err(|e| GitError::CommandFailed(format!("Failed to find origin remote: {e}")))?;

    let config = repo
        .config()
        .map_err(|e| GitError::CommandFailed(format!("Failed to get git config: {e}")))?;

    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(|url, username_from_url, allowed_types| {
        // Try SSH agent first for SSH URLs
        if allowed_types.contains(git2::CredentialType::SSH_KEY)
            && let Some(username) = username_from_url
        {
            return Cred::ssh_key_from_agent(username);
        }

        // For HTTPS, use git2's native credential helper support
        if allowed_types.contains(git2::CredentialType::USER_PASS_PLAINTEXT)
            && let Ok(cred) = Cred::credential_helper(&config, url, username_from_url)
        {
            return Ok(cred);
        }

        // Fallback to default credentials
        Cred::default()
    });

    let mut fetch_opts = FetchOptions::new();
    fetch_opts.prune(git2::FetchPrune::On);
    fetch_opts.remote_callbacks(callbacks);

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
    use rstest::rstest;

    /// Helper to create remote tracking branch references in a test repo.
    fn create_remote_branches(repo: &Repository, branches: &[&str]) {
        let head_commit = repo.head().unwrap().peel_to_commit().unwrap();
        for branch in branches {
            repo.reference(
                &format!("refs/remotes/origin/{branch}"),
                head_commit.id(),
                true,
                "create fake remote branch for test",
            )
            .unwrap();
        }
    }

    #[rstest]
    #[case::only_origin_main(vec!["main"], Some("main"))]
    #[case::only_origin_master(vec!["master"], Some("master"))]
    #[case::both_prefers_main(vec!["main", "master"], Some("main"))]
    #[case::no_remote_branches(vec![], None)]
    fn test_get_main_branch_for_repo(
        #[case] remote_branches: Vec<&str>,
        #[case] expected: Option<&str>,
    ) {
        let temp = TempRepo::new("owner", "repo", "master");
        let repo = temp.open();

        create_remote_branches(&repo, &remote_branches);

        let result = get_main_branch_for_repo(&repo);
        match expected {
            Some(branch) => assert_eq!(result.unwrap(), branch),
            None => assert!(result.is_err()),
        }
    }
}
