use std::path::PathBuf;

use clap::Args;
use git2::Repository;

use super::error::{Result, WmError};

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ListArgs {}

pub fn run(_args: &ListArgs) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let repo = Repository::open_from_env().map_err(|_| WmError::NotInGitRepo)?;
    let entries = list_worktrees(&repo)?;
    for entry in entries {
        println!(
            "{:<50} {} [{}]",
            entry.path.display(),
            entry.commit,
            entry.branch
        );
    }
    Ok(())
}

/// Information about a worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    pub branch: String,
    pub commit: String,
}

/// List all worktrees (main + linked) for a repository.
pub fn list_worktrees(repo: &Repository) -> Result<Vec<WorktreeInfo>> {
    let mut entries = Vec::new();

    // Get the main worktree path
    let main_path = if repo.is_worktree() {
        repo.commondir()
            .parent()
            .ok_or(WmError::NotInGitRepo)?
            .to_path_buf()
    } else {
        repo.workdir().ok_or(WmError::NotInGitRepo)?.to_path_buf()
    };

    // Add main worktree
    let head = repo.head().ok();
    let main_branch = head
        .as_ref()
        .and_then(|h| h.shorthand())
        .unwrap_or("(unknown)")
        .to_string();
    let main_commit = head
        .as_ref()
        .and_then(|h| h.peel_to_commit().ok())
        .map(|c| c.id().to_string())
        .map(|s| s[..7].to_string())
        .unwrap_or_else(|| "(none)".to_string());

    entries.push(WorktreeInfo {
        path: main_path,
        branch: main_branch,
        commit: main_commit,
    });

    // List linked worktrees
    let worktrees = repo
        .worktrees()
        .map_err(|e| WmError::CommandFailed(e.message().to_string()))?;
    for name in worktrees.iter().flatten() {
        if let Ok(wt) = repo.find_worktree(name) {
            let wt_path = wt.path().to_path_buf();
            // Open the worktree repository to get its HEAD
            if let Ok(wt_repo) = Repository::open(&wt_path) {
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
                entries.push(WorktreeInfo {
                    path: wt_path,
                    branch,
                    commit,
                });
            }
        }
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestRepo;

    #[test]
    fn list_worktrees_returns_main_repo() {
        let repo = TestRepo::new();
        let entries = list_worktrees(&repo.open()).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, repo.path());
        assert_eq!(entries[0].branch, "master");
        assert_eq!(entries[0].commit.len(), 7);
    }

    #[test]
    fn list_worktrees_includes_linked_worktrees() {
        let repo = TestRepo::new();
        repo.create_worktree("feature-branch");

        let entries = list_worktrees(&repo.open()).unwrap();

        assert_eq!(entries.len(), 2);
        // Main repo
        assert_eq!(entries[0].path, repo.path());
        // Linked worktree
        assert_eq!(entries[1].path, repo.worktree_path("feature-branch"));
        assert_eq!(entries[1].branch, "feature-branch");
    }

    #[test]
    fn list_worktrees_from_worktree_lists_all() {
        let repo = TestRepo::new();
        repo.create_worktree("feature-branch");

        // Open from the worktree instead of main repo
        let wt_repo = Repository::open(repo.worktree_path("feature-branch")).unwrap();
        let entries = list_worktrees(&wt_repo).unwrap();

        // Should still list both worktrees
        assert_eq!(entries.len(), 2);
    }
}
