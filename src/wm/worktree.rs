//! Common worktree operations shared between delete and clean commands.

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
