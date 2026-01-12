//! Common worktree operations shared between delete, clean, and list commands.

use std::path::PathBuf;

use git2::{BranchType, Repository, WorktreePruneOptions};

use super::error::{Result, WmError};
use super::git::local_branch_exists;

/// Get the main repository, resolving from a worktree if necessary.
pub fn get_main_repo(repo: &Repository) -> Result<Repository> {
    if repo.is_worktree() {
        let commondir = repo.commondir();
        Repository::open(commondir.parent().ok_or(WmError::NotInGitRepo)?)
            .map_err(|_| WmError::NotInGitRepo)
    } else {
        // Clone isn't available, so re-open from workdir
        let workdir = repo.workdir().ok_or(WmError::NotInGitRepo)?;
        Repository::open(workdir).map_err(|_| WmError::NotInGitRepo)
    }
}

/// Get the branch name associated with a worktree.
pub fn get_worktree_branch(repo: &Repository, worktree_name: &str) -> Option<String> {
    let worktree = repo.find_worktree(worktree_name).ok()?;
    let wt_repo = Repository::open_from_worktree(&worktree).ok()?;
    let head = wt_repo.head().ok()?;
    head.shorthand().map(|s| s.to_string())
}

/// Delete a worktree by name. Returns true if successful.
pub fn delete_worktree(repo: &Repository, worktree_name: &str) -> Result<bool> {
    let worktree = match repo.find_worktree(worktree_name) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("Failed to find worktree {worktree_name}: {e}");
            return Ok(false);
        }
    };

    let mut prune_opts = WorktreePruneOptions::new();
    prune_opts.valid(true).working_tree(true);

    match worktree.prune(Some(&mut prune_opts)) {
        Ok(()) => Ok(true),
        Err(e) => {
            eprintln!("Failed to delete worktree {worktree_name}: {e}");
            Ok(false)
        }
    }
}

/// Delete a local branch if it exists. Returns true if deleted.
pub fn delete_branch_if_exists(repo: &Repository, branch: &str) -> bool {
    if branch.is_empty() || !local_branch_exists(branch) {
        return false;
    }

    if let Ok(mut branch_ref) = repo.find_branch(branch, BranchType::Local) {
        branch_ref.delete().is_ok()
    } else {
        false
    }
}

/// Find the worktree name from its path.
pub fn find_worktree_name(repo: &Repository, worktree_path: &str) -> Result<String> {
    let worktrees = repo
        .worktrees()
        .map_err(|e| WmError::CommandFailed(e.message().to_string()))?;

    for name in worktrees.iter().flatten() {
        if let Ok(wt) = repo.find_worktree(name) {
            let wt_path = wt.path().to_string_lossy();
            let wt_path_normalized = wt_path.trim_end_matches('/');
            let worktree_path_normalized = worktree_path.trim_end_matches('/');
            if wt_path_normalized == worktree_path_normalized {
                return Ok(name.to_string());
            }
        }
    }

    Err(WmError::WorktreeNotFound(worktree_path.to_string()))
}

/// Basic information about a linked worktree.
#[derive(Debug, Clone)]
pub struct LinkedWorktree {
    /// The worktree name (used by git internally).
    pub name: String,
    /// The worktree path on disk.
    pub path: PathBuf,
    /// The branch name checked out in this worktree.
    pub branch: String,
    /// The short commit hash (7 chars).
    pub commit: String,
}

/// List all linked worktrees (excludes main worktree).
pub fn list_linked_worktrees(repo: &Repository) -> Result<Vec<LinkedWorktree>> {
    let worktrees = repo
        .worktrees()
        .map_err(|e| WmError::CommandFailed(e.message().to_string()))?;

    let mut result = Vec::new();

    for name in worktrees.iter().flatten() {
        let wt = match repo.find_worktree(name) {
            Ok(w) => w,
            Err(_) => continue,
        };

        let wt_path = wt.path().to_path_buf();

        // Open the worktree repository to get its HEAD
        let wt_repo = match Repository::open(&wt_path) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let wt_head = wt_repo.head().ok();
        let branch = wt_head
            .as_ref()
            .and_then(|h| h.shorthand())
            .unwrap_or("(unknown)")
            .to_string();
        let commit = wt_head
            .as_ref()
            .and_then(|h| h.peel_to_commit().ok())
            .map(|c| c.id().to_string())
            .map(|s| s[..7].to_string())
            .unwrap_or_else(|| "(none)".to_string());

        result.push(LinkedWorktree {
            name: name.to_string(),
            path: wt_path,
            branch,
            commit,
        });
    }

    Ok(result)
}

/// Get the main worktree path.
pub fn get_main_worktree_path(repo: &Repository) -> Result<PathBuf> {
    if repo.is_worktree() {
        repo.commondir()
            .parent()
            .map(|p| p.to_path_buf())
            .ok_or(WmError::NotInGitRepo)
    } else {
        repo.workdir()
            .map(|p| p.to_path_buf())
            .ok_or(WmError::NotInGitRepo)
    }
}

/// Get the branch and commit for the main worktree.
pub fn get_main_worktree_info(repo: &Repository) -> (String, String) {
    let head = repo.head().ok();
    let branch = head
        .as_ref()
        .and_then(|h| h.shorthand())
        .unwrap_or("(unknown)")
        .to_string();
    let commit = head
        .as_ref()
        .and_then(|h| h.peel_to_commit().ok())
        .map(|c| c.id().to_string())
        .map(|s| s[..7].to_string())
        .unwrap_or_else(|| "(none)".to_string());
    (branch, commit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestRepo;

    #[test]
    fn get_main_repo_from_main_returns_same_repo() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        let main_repo = get_main_repo(&repo).unwrap();
        assert_eq!(
            main_repo.workdir().unwrap().canonicalize().unwrap(),
            test_repo.path()
        );
    }

    #[test]
    fn get_main_repo_from_worktree_returns_main() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("feature");

        let wt_repo = Repository::open(test_repo.worktree_path("feature")).unwrap();
        let main_repo = get_main_repo(&wt_repo).unwrap();

        assert_eq!(
            main_repo.workdir().unwrap().canonicalize().unwrap(),
            test_repo.path()
        );
    }

    #[test]
    fn get_worktree_branch_returns_branch_name() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("feature-branch");

        let repo = test_repo.open();
        let branch = get_worktree_branch(&repo, "feature-branch");

        assert_eq!(branch, Some("feature-branch".to_string()));
    }

    #[test]
    fn get_worktree_branch_returns_none_for_nonexistent() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        let branch = get_worktree_branch(&repo, "nonexistent");
        assert_eq!(branch, None);
    }

    #[test]
    fn delete_worktree_removes_worktree() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("to-delete");

        let repo = test_repo.open();

        // Verify worktree exists
        assert!(repo.find_worktree("to-delete").is_ok());

        // Delete it
        let result = delete_worktree(&repo, "to-delete").unwrap();
        assert!(result);

        // Verify it's gone
        assert!(repo.find_worktree("to-delete").is_err());
    }

    #[test]
    fn delete_worktree_returns_false_for_nonexistent() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        let result = delete_worktree(&repo, "nonexistent").unwrap();
        assert!(!result);
    }

    #[test]
    fn find_worktree_name_finds_by_path() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("my-feature");

        let repo = test_repo.open();
        let wt_path = test_repo.worktree_path("my-feature");

        let name = find_worktree_name(&repo, wt_path.to_str().unwrap()).unwrap();
        assert_eq!(name, "my-feature");
    }

    #[test]
    fn find_worktree_name_returns_error_for_nonexistent() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        let result = find_worktree_name(&repo, "/nonexistent/path");
        assert!(result.is_err());
    }

    #[test]
    fn list_linked_worktrees_empty_when_no_worktrees() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        let worktrees = list_linked_worktrees(&repo).unwrap();
        assert!(worktrees.is_empty());
    }

    #[test]
    fn list_linked_worktrees_returns_all_worktrees() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("feature-a");
        test_repo.create_worktree("feature-b");

        let repo = test_repo.open();
        let worktrees = list_linked_worktrees(&repo).unwrap();

        assert_eq!(worktrees.len(), 2);

        let names: Vec<&str> = worktrees.iter().map(|w| w.name.as_str()).collect();
        assert!(names.contains(&"feature-a"));
        assert!(names.contains(&"feature-b"));
    }

    #[test]
    fn list_linked_worktrees_includes_branch_and_commit() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("feature");

        let repo = test_repo.open();
        let worktrees = list_linked_worktrees(&repo).unwrap();

        assert_eq!(worktrees.len(), 1);
        let wt = &worktrees[0];
        assert_eq!(wt.name, "feature");
        assert_eq!(wt.branch, "feature");
        assert_eq!(wt.commit.len(), 7);
        assert_eq!(wt.path, test_repo.worktree_path("feature"));
    }

    #[test]
    fn get_main_worktree_path_from_main() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        let path = get_main_worktree_path(&repo).unwrap();
        assert_eq!(path.canonicalize().unwrap(), test_repo.path());
    }

    #[test]
    fn get_main_worktree_path_from_worktree() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("feature");

        let wt_repo = Repository::open(test_repo.worktree_path("feature")).unwrap();
        let path = get_main_worktree_path(&wt_repo).unwrap();

        assert_eq!(path.canonicalize().unwrap(), test_repo.path());
    }

    #[test]
    fn get_main_worktree_info_returns_branch_and_commit() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        let (branch, commit) = get_main_worktree_info(&repo);

        assert_eq!(branch, "master");
        assert_eq!(commit.len(), 7);
    }
}
