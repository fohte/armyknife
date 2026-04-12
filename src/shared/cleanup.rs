//! Shared cleanup logic for Claude Code sessions and git worktrees.
//!
//! Both `cc watch` (session deletion) and `wm delete`/`wm clean` (worktree deletion)
//! need to clean up related resources. This module provides the shared logic to
//! ensure consistent cleanup regardless of the entry point.

use std::path::Path;

use anyhow::Context;
use git2::Repository;

use crate::commands::cc::store;
use crate::infra::tmux;

/// Result of worktree resource cleanup.
#[derive(Debug, Default)]
pub struct WorktreeCleanupResult {
    /// Whether a git worktree was deleted.
    pub worktree_deleted: bool,
    /// Name of the branch that was deleted, if any.
    pub branch_deleted: Option<String>,
    /// Number of tmux windows that were closed.
    pub windows_closed: usize,
}

/// Cleans up worktree resources for a given working directory.
///
/// If `cwd` is inside a git worktree (not the main working tree), this function:
/// 1. Deletes the git worktree
/// 2. Deletes the associated branch
/// 3. Closes tmux windows whose panes are inside the worktree path
///
/// Returns a result describing what was cleaned up, or a default result if
/// `cwd` is not a worktree.
pub fn cleanup_worktree_resources(cwd: &Path) -> anyhow::Result<WorktreeCleanupResult> {
    // Avoid importing wm::worktree directly; use git2 API to check worktree status
    let repo = match Repository::open(cwd) {
        Ok(r) => r,
        Err(_) => return Ok(WorktreeCleanupResult::default()),
    };

    if !repo.is_worktree() {
        return Ok(WorktreeCleanupResult::default());
    }

    // Navigate to main repo to perform worktree operations
    let commondir = repo.commondir();
    let main_repo = Repository::open(
        commondir
            .parent()
            .context("Failed to resolve main repo path from commondir")?,
    )
    .context("Failed to open main repository")?;

    // Find the worktree name from the path
    let cwd_str = cwd.to_string_lossy();
    let worktree_name = find_worktree_name_for_path(&main_repo, &cwd_str)?;

    let Some(worktree_name) = worktree_name else {
        return Ok(WorktreeCleanupResult::default());
    };

    // Get branch name before deleting the worktree
    let branch_name = get_worktree_branch(&main_repo, &worktree_name);

    // Collect tmux window IDs before deleting the worktree
    let window_ids = tmux::get_window_ids_in_path(&cwd_str);

    // Delete the worktree
    let worktree_deleted = delete_git_worktree(&main_repo, &worktree_name)?;

    // Delete the branch
    let branch_deleted = if worktree_deleted {
        branch_name.filter(|branch| delete_branch_if_exists(&main_repo, branch))
    } else {
        None
    };

    // Close tmux windows (best-effort)
    let mut windows_closed = 0;
    for window_id in &window_ids {
        if tmux::kill_window(window_id).is_ok() {
            windows_closed += 1;
        }
    }

    Ok(WorktreeCleanupResult {
        worktree_deleted,
        branch_deleted,
        windows_closed,
    })
}

/// Cleans up Claude Code session files for sessions whose `cwd` is inside `worktree_path`.
///
/// For each matching session:
/// 1. If the session has a tmux pane, sends SIGTERM to it
/// 2. Deletes the session file
///
/// Returns the number of sessions cleaned up.
pub fn cleanup_sessions_in_path(worktree_path: &Path) -> anyhow::Result<usize> {
    let sessions = store::list_sessions()?;
    let mut cleaned = 0;

    for session in &sessions {
        if session.cwd.starts_with(worktree_path) {
            // Send SIGTERM to alive sessions
            if let Some(ref tmux_info) = session.tmux_info
                && tmux::is_pane_alive(&tmux_info.pane_id)
            {
                tmux::send_sigterm_to_pane(&tmux_info.pane_id);
            }

            // Delete the session file (best-effort, log errors)
            if let Err(e) = store::delete_session(&session.session_id) {
                eprintln!(
                    "Warning: Failed to delete session {}: {e}",
                    session.session_id
                );
            } else {
                cleaned += 1;
            }
        }
    }

    Ok(cleaned)
}

// ============================================================================
// Internal helpers - duplicated from wm::worktree to avoid cross-command deps
// ============================================================================

/// Find worktree name by matching path against all worktrees.
fn find_worktree_name_for_path(repo: &Repository, path: &str) -> anyhow::Result<Option<String>> {
    let worktrees = repo.worktrees().context("Failed to list worktrees")?;

    for name in worktrees.iter().flatten() {
        if let Ok(wt) = repo.find_worktree(name) {
            let wt_path = wt.path().to_string_lossy();
            let wt_path_normalized = wt_path.trim_end_matches('/');
            let path_normalized = path.trim_end_matches('/');
            if wt_path_normalized == path_normalized {
                return Ok(Some(name.to_string()));
            }
        }
    }

    Ok(None)
}

/// Get the branch name for a worktree.
fn get_worktree_branch(repo: &Repository, worktree_name: &str) -> Option<String> {
    let worktree = repo.find_worktree(worktree_name).ok()?;
    let wt_repo = Repository::open_from_worktree(&worktree).ok()?;
    let head = wt_repo.head().ok()?;
    head.shorthand().map(|s| s.to_string())
}

/// Delete a git worktree by name.
fn delete_git_worktree(repo: &Repository, worktree_name: &str) -> anyhow::Result<bool> {
    let worktree = match repo.find_worktree(worktree_name) {
        Ok(w) => w,
        Err(_) => return Ok(false),
    };

    let mut prune_opts = git2::WorktreePruneOptions::new();
    prune_opts.valid(true).working_tree(true);

    match worktree.prune(Some(&mut prune_opts)) {
        Ok(()) => Ok(true),
        Err(e) => {
            eprintln!("Failed to delete worktree {worktree_name}: {e}");
            Ok(false)
        }
    }
}

/// Delete a local branch if it exists.
fn delete_branch_if_exists(repo: &Repository, branch: &str) -> bool {
    if branch.is_empty() {
        return false;
    }
    if let Ok(mut branch_ref) = repo.find_branch(branch, git2::BranchType::Local) {
        branch_ref.delete().is_ok()
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::testing::TestRepo;

    #[test]
    fn find_worktree_name_for_path_finds_existing() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("feature-x");

        let repo = test_repo.open();
        let wt_path = test_repo.worktree_path("feature-x");

        let name =
            find_worktree_name_for_path(&repo, wt_path.to_str().unwrap()).expect("should succeed");
        assert_eq!(name, Some("feature-x".to_string()));
    }

    #[test]
    fn find_worktree_name_for_path_returns_none_for_nonexistent() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        let name = find_worktree_name_for_path(&repo, "/nonexistent/path").expect("should succeed");
        assert_eq!(name, None);
    }

    #[test]
    fn cleanup_worktree_resources_on_non_worktree_returns_default() {
        let test_repo = TestRepo::new();
        let result =
            cleanup_worktree_resources(&test_repo.path()).expect("should succeed on main repo");

        assert!(!result.worktree_deleted);
        assert!(result.branch_deleted.is_none());
        assert_eq!(result.windows_closed, 0);
    }

    #[test]
    fn cleanup_worktree_resources_on_nonexistent_path_returns_default() {
        let result = cleanup_worktree_resources(Path::new("/nonexistent/path/to/repo"))
            .expect("should succeed on missing path");

        assert!(!result.worktree_deleted);
        assert!(result.branch_deleted.is_none());
        assert_eq!(result.windows_closed, 0);
    }

    #[test]
    fn cleanup_worktree_resources_deletes_worktree_and_branch() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("cleanup-test");

        let wt_path = test_repo.worktree_path("cleanup-test");
        let result = cleanup_worktree_resources(&wt_path).expect("should succeed");

        assert!(result.worktree_deleted);
        assert_eq!(result.branch_deleted, Some("cleanup-test".to_string()));

        // Verify worktree is gone
        let repo = test_repo.open();
        assert!(repo.find_worktree("cleanup-test").is_err());
    }
}
