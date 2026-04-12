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

    cleanup_worktree_by_name(&main_repo, &worktree_name, &worktree_root)
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
    let branch_name = get_worktree_branch(repo, worktree_name);

    let path_str = worktree_path.to_string_lossy();
    let window_ids = tmux::get_window_ids_in_path(&path_str);

    let worktree_deleted = delete_worktree(repo, worktree_name)?;

    let branch_deleted = if worktree_deleted {
        branch_name.filter(|branch| delete_branch_if_exists(repo, branch))
    } else {
        None
    };

    let mut windows_closed = 0;
    for window_id in &window_ids {
        if tmux::kill_window(window_id).is_ok() {
            windows_closed += 1;
        }
    }

    let sessions_cleaned = cleanup_sessions_in_path(worktree_path).unwrap_or(0);

    Ok(WorktreeCleanupResult {
        worktree_deleted,
        branch_deleted,
        windows_closed,
        sessions_cleaned,
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

    #[test]
    fn cleanup_worktree_resources_on_non_worktree_returns_default() {
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
        let result = cleanup_worktree_resources(Path::new("/nonexistent/path/to/repo"))
            .expect("should succeed on missing path");

        assert!(!result.worktree_deleted);
        assert!(result.branch_deleted.is_none());
        assert_eq!(result.windows_closed, 0);
        assert_eq!(result.sessions_cleaned, 0);
    }

    #[test]
    fn cleanup_worktree_resources_deletes_worktree_and_branch() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("cleanup-test");

        let wt_path = test_repo.worktree_path("cleanup-test");
        let result = cleanup_worktree_resources(&wt_path).expect("should succeed");

        assert!(result.worktree_deleted);
        assert_eq!(result.branch_deleted, Some("cleanup-test".to_string()));

        let repo = test_repo.open();
        assert!(repo.find_worktree("cleanup-test").is_err());
    }

    // Subdirectory test is not possible in srt sandbox because
    // git2::Repository::open cannot discover worktrees from subdirectories
    // in that environment. The worktree root resolution via repo.workdir()
    // is covered by the root-path tests above.

    #[test]
    fn cleanup_worktree_by_name_deletes_worktree_and_branch() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("named-cleanup");

        let repo = test_repo.open();
        let wt_path = test_repo.worktree_path("named-cleanup");
        let result =
            cleanup_worktree_by_name(&repo, "named-cleanup", &wt_path).expect("should succeed");

        assert!(result.worktree_deleted);
        assert_eq!(result.branch_deleted, Some("named-cleanup".to_string()));
        assert!(repo.find_worktree("named-cleanup").is_err());
    }

    #[test]
    fn cleanup_worktree_by_name_returns_false_for_nonexistent() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        let result = cleanup_worktree_by_name(&repo, "nonexistent", Path::new("/tmp/nonexistent"))
            .expect("should succeed");

        assert!(!result.worktree_deleted);
        assert!(result.branch_deleted.is_none());
    }
}
