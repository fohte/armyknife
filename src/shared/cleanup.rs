//! Shared cleanup logic for Claude Code sessions and git worktrees.
//!
//! Both `cc watch` (session deletion) and `wm delete`/`wm clean` (worktree deletion)
//! need to clean up related resources. This module provides the shared logic to
//! ensure consistent cleanup regardless of the entry point.

use std::path::Path;

use git2::Repository;

use crate::commands::cc::store;
use crate::commands::wm::worktree::{
    delete_branch_if_exists, delete_worktree, find_worktree_name, get_main_repo,
    get_worktree_branch,
};
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
    /// Number of Claude Code sessions cleaned up.
    pub sessions_cleaned: usize,
    /// The resolved worktree root path (set when worktree_deleted is true).
    /// Use this instead of raw cwd for path matching, since cwd may be a
    /// subdirectory.
    pub worktree_root: Option<std::path::PathBuf>,
}

/// Cleans up all resources associated with a worktree at `cwd`:
/// worktree itself, branch, tmux windows, and Claude Code session files.
///
/// `cwd` can be any path inside the worktree (including subdirectories);
/// the worktree root is resolved via `repo.workdir()`.
///
/// If `cwd` is not inside a git worktree, returns a default (no-op) result.
pub fn cleanup_worktree_resources(cwd: &Path) -> anyhow::Result<WorktreeCleanupResult> {
    let repo = match Repository::open(cwd) {
        Ok(r) => r,
        Err(_) => return Ok(WorktreeCleanupResult::default()),
    };

    if !repo.is_worktree() {
        return Ok(WorktreeCleanupResult::default());
    }

    let main_repo = get_main_repo(&repo)?;

    // Resolve the worktree root directory, not the raw cwd which may be
    // a subdirectory. This ensures find_worktree_name matches correctly.
    let worktree_root = match repo.workdir() {
        Some(p) => p.to_path_buf(),
        None => return Ok(WorktreeCleanupResult::default()),
    };
    let worktree_root_str = worktree_root.to_string_lossy();

    let worktree_name = match find_worktree_name(&main_repo, &worktree_root_str) {
        Ok(name) => name,
        Err(_) => return Ok(WorktreeCleanupResult::default()),
    };

    let mut result = cleanup_worktree_by_name(&main_repo, &worktree_name, &worktree_root)?;
    if result.worktree_deleted {
        result.worktree_root = Some(worktree_root);
    }
    Ok(result)
}

/// Cleans up all resources for a worktree identified by `repo` and `worktree_name`:
/// worktree itself, branch, tmux windows, and Claude Code session files.
///
/// `worktree_path` is the filesystem path of the worktree root, used for
/// tmux window and session file lookup.
pub fn cleanup_worktree_by_name(
    repo: &Repository,
    worktree_name: &str,
    worktree_path: &Path,
) -> anyhow::Result<WorktreeCleanupResult> {
    // Collect tmux window IDs before deleting the worktree (paths still exist)
    let path_str = worktree_path.to_string_lossy();
    let window_ids = tmux::get_window_ids_in_path(&path_str);

    let mut result = delete_worktree_and_branch(repo, worktree_name);

    // Only clean up tmux windows and sessions if worktree deletion succeeded
    if result.worktree_deleted {
        for window_id in &window_ids {
            if tmux::kill_window(window_id).is_ok() {
                result.windows_closed += 1;
            }
        }

        result.sessions_cleaned = cleanup_sessions_in_path(worktree_path).unwrap_or(0);
    }

    Ok(result)
}

/// Deletes a git worktree and its associated branch.
/// Pure git2 operation with no external command dependencies.
fn delete_worktree_and_branch(repo: &Repository, worktree_name: &str) -> WorktreeCleanupResult {
    let branch_name = get_worktree_branch(repo, worktree_name);

    let worktree_deleted = delete_worktree(repo, worktree_name).unwrap_or(false);

    let branch_deleted = if worktree_deleted {
        branch_name.filter(|branch| delete_branch_if_exists(repo, branch))
    } else {
        None
    };

    WorktreeCleanupResult {
        worktree_deleted,
        branch_deleted,
        ..WorktreeCleanupResult::default()
    }
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

    // Batch-fetch alive pane IDs to avoid per-session tmux process spawning
    let alive_panes = tmux::list_all_pane_ids().unwrap_or_default();

    for session in &sessions {
        if session.cwd.starts_with(worktree_path) {
            if let Some(ref tmux_info) = session.tmux_info
                && alive_panes.contains(&tmux_info.pane_id)
            {
                tmux::send_sigterm_to_pane(&tmux_info.pane_id);
            }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::testing::TestRepo;

    // Tests exercise delete_worktree_and_branch (pure git2 operations) to
    // avoid depending on external commands like tmux or session store I/O.

    #[test]
    fn delete_worktree_and_branch_deletes_both() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("cleanup-test");

        let repo = test_repo.open();
        let result = delete_worktree_and_branch(&repo, "cleanup-test");

        assert!(result.worktree_deleted);
        assert_eq!(result.branch_deleted, Some("cleanup-test".to_string()));
        assert_eq!(result.windows_closed, 0);
        assert_eq!(result.sessions_cleaned, 0);

        assert!(repo.find_worktree("cleanup-test").is_err());
    }

    #[test]
    fn delete_worktree_and_branch_returns_false_for_nonexistent() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        let result = delete_worktree_and_branch(&repo, "nonexistent");

        assert!(!result.worktree_deleted);
        assert!(result.branch_deleted.is_none());
    }

    #[test]
    fn cleanup_worktree_resources_on_non_worktree_returns_default() {
        // Repository::open on a non-worktree returns is_worktree() == false,
        // so no external commands are invoked.
        let test_repo = TestRepo::new();
        let result =
            cleanup_worktree_resources(&test_repo.path()).expect("should succeed on main repo");

        assert!(!result.worktree_deleted);
        assert!(result.branch_deleted.is_none());
        assert_eq!(result.windows_closed, 0);
        assert_eq!(result.sessions_cleaned, 0);
    }

    #[test]
    fn cleanup_worktree_resources_on_nonexistent_path_returns_default() {
        // Repository::open fails, so no external commands are invoked.
        let result = cleanup_worktree_resources(Path::new("/nonexistent/path/to/repo"))
            .expect("should succeed on missing path");

        assert!(!result.worktree_deleted);
        assert!(result.branch_deleted.is_none());
        assert_eq!(result.windows_closed, 0);
        assert_eq!(result.sessions_cleaned, 0);
    }
}
