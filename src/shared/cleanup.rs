//! Shared cleanup logic for Claude Code sessions, notifications, and git worktrees.
//!
//! Both `agent watch` (session deletion) and `wm delete`/`wm clean` (worktree deletion)
//! need to clean up related resources. This module provides the shared logic to
//! ensure consistent cleanup regardless of the entry point.

use std::collections::HashSet;
use std::path::Path;

use crate::commands::agent::store;
use crate::commands::agent::types::{Session, SessionStatus};
use crate::commands::wm::worktree::{
    delete_branch_if_exists, delete_worktree, find_worktree_name, get_main_repo,
    get_worktree_branch,
};
use crate::infra::git::GitRepo;
use crate::infra::process;
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
    /// Number of process groups killed that were rooted in the worktree
    /// (identified by a member process whose cwd was inside it).
    pub process_groups_signaled: usize,
    /// The resolved worktree root path (set when worktree_deleted is true).
    /// Use this instead of raw cwd for path matching, since cwd may be a
    /// subdirectory.
    pub worktree_root: Option<std::path::PathBuf>,
}

/// Cleans up all resources associated with a worktree at `cwd`:
/// worktree itself, branch, tmux windows, Claude Code session files, and notifications.
///
/// `cwd` can be any path inside the worktree (including subdirectories);
/// the worktree root is resolved via `repo.workdir()`.
///
/// If `cwd` is not inside a git worktree, returns a default (no-op) result.
pub fn cleanup_worktree_resources(cwd: &Path) -> anyhow::Result<WorktreeCleanupResult> {
    let repo = match GitRepo::open_at(cwd) {
        Ok(r) => r,
        Err(_) => return Ok(WorktreeCleanupResult::default()),
    };

    if !repo.is_worktree() {
        return Ok(WorktreeCleanupResult::default());
    }

    let main_repo = get_main_repo(&repo)?;

    let worktree_root = repo.workdir().to_path_buf();
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
/// worktree itself, branch, tmux windows, Claude Code session files, and notifications.
///
/// `worktree_path` is the filesystem path of the worktree root, used for
/// tmux window and session file lookup.
pub fn cleanup_worktree_by_name(
    repo: &GitRepo,
    worktree_name: &str,
    worktree_path: &Path,
) -> anyhow::Result<WorktreeCleanupResult> {
    // Collect tmux window IDs and process groups rooted in the worktree
    // before deleting it (the path must still exist to match against).
    let path_str = worktree_path.to_string_lossy();
    let window_ids = tmux::get_window_ids_in_path(&path_str);
    let orphan_pgids = process::find_pgids_in_path(worktree_path);

    let mut result = delete_worktree_and_branch(repo, worktree_name);

    // Only clean up tmux windows, sessions, and processes if worktree
    // deletion succeeded.
    //
    // Clean up sessions BEFORE killing tmux windows: if the caller is running
    // inside one of those windows (e.g. `a wm delete` invoked from the
    // worktree's own pane), kill_window terminates the caller's pane and
    // SIGHUPs this very process, leaving Paused sessions orphaned on disk.
    if result.worktree_deleted {
        result.sessions_cleaned = cleanup_sessions_in_path(worktree_path).unwrap_or(0);
        result.process_groups_signaled = process::kill_process_groups(&orphan_pgids);

        for window_id in &window_ids {
            if tmux::kill_window(window_id).is_ok() {
                result.windows_closed += 1;
            }
        }
    }

    Ok(result)
}

/// Deletes a git worktree and its associated branch.
fn delete_worktree_and_branch(repo: &GitRepo, worktree_name: &str) -> WorktreeCleanupResult {
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

/// Returns true if the pane hosting this session still runs a live Claude
/// process that needs SIGTERM before we drop the session file.
///
/// Paused sessions were already SIGTERM'd by `agent sweep`, and Ended sessions
/// exited on their own. In both cases the pane's foreground process is the
/// user's shell (possibly the shell that just invoked `a wm delete` from
/// within the worktree). Signaling that shell would kill the very process we
/// are running in, so restrict SIGTERM to statuses where a Claude process is
/// still expected to be alive.
fn should_sigterm_session(status: SessionStatus) -> bool {
    match status {
        SessionStatus::Running | SessionStatus::WaitingInput | SessionStatus::Stopped => true,
        SessionStatus::Paused | SessionStatus::Ended => false,
    }
}

/// Cleans up active Claude Code session files and notification groups for sessions
/// whose `cwd` is inside `worktree_path`.
///
/// Every stored status is considered so `Ended` sessions with stale notifications
/// are included. Ended session records are retained for later delegated-session lookup.
///
/// For each matching session:
/// 1. If the session's Claude process is still expected to be alive and the
///    tmux pane is alive, sends SIGTERM to it
/// 2. Deletes the session file unless the session has already ended
/// 3. Removes the notification group on a best-effort basis
///
/// Returns the number of sessions cleaned up.
pub fn cleanup_sessions_in_path(worktree_path: &Path) -> anyhow::Result<usize> {
    let sessions = store::list_all_sessions()?;
    let alive_panes = tmux::list_all_pane_ids().unwrap_or_default();

    Ok(cleanup_sessions(
        &sessions,
        worktree_path,
        &alive_panes,
        tmux::send_sigterm_to_pane,
        store::delete_session,
        crate::infra::notification::remove_group,
    ))
}

fn cleanup_sessions(
    sessions: &[Session],
    worktree_path: &Path,
    alive_panes: &HashSet<String>,
    mut send_sigterm: impl FnMut(&str),
    mut delete_session: impl FnMut(&str) -> anyhow::Result<()>,
    mut remove_notification_group: impl FnMut(&str) -> anyhow::Result<()>,
) -> usize {
    let mut cleaned = 0;

    for session in sessions {
        if session.cwd.starts_with(worktree_path) {
            if should_sigterm_session(session.status)
                && let Some(ref tmux_info) = session.tmux_info
                && alive_panes.contains(&tmux_info.pane_id)
            {
                send_sigterm(&tmux_info.pane_id);
            }

            if session.status != SessionStatus::Ended {
                if let Err(e) = delete_session(&session.session_id) {
                    eprintln!(
                        "Warning: Failed to delete session {}: {e}",
                        session.session_id
                    );
                } else {
                    cleaned += 1;
                }
            }

            if let Err(e) = remove_notification_group(&session.session_id) {
                eprintln!(
                    "Warning: Failed to remove notification group for session {}: {e}",
                    session.session_id
                );
                tracing::warn!(
                    target: "armyknife::shared::cleanup",
                    event = "cleanup.notification.err",
                    session = %session.session_id,
                    msg = format!("failed to remove notification group: {e}"),
                );
            }
        }
    }

    cleaned
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::testing::TestRepo;
    use rstest::rstest;

    // Tests exercise delete_worktree_and_branch directly to avoid depending
    // on tmux or session store I/O.

    #[rstest]
    #[case::running(SessionStatus::Running, true)]
    #[case::waiting(SessionStatus::WaitingInput, true)]
    #[case::stopped(SessionStatus::Stopped, true)]
    #[case::paused(SessionStatus::Paused, false)]
    #[case::ended(SessionStatus::Ended, false)]
    fn should_sigterm_session_by_status(#[case] status: SessionStatus, #[case] expected: bool) {
        // Paused sessions were already SIGTERM'd by `agent sweep` so the pane
        // now hosts the user's shell -- signaling it would kill the caller
        // when `a wm delete` runs from that same pane.
        assert_eq!(should_sigterm_session(status), expected);
    }

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
        assert_eq!(result.process_groups_signaled, 0);

        // After deletion, the worktree should no longer be listed.
        let list =
            crate::infra::git::cmd::run_git(repo.workdir(), ["worktree", "list", "--porcelain"])
                .unwrap();
        assert!(!list.contains("/.worktrees/cleanup-test"));
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
        // Opening a non-worktree path returns is_worktree() == false, so no
        // tmux commands run.
        let test_repo = TestRepo::new();
        let result =
            cleanup_worktree_resources(&test_repo.path()).expect("should succeed on main repo");

        assert!(!result.worktree_deleted);
        assert!(result.branch_deleted.is_none());
        assert_eq!(result.windows_closed, 0);
        assert_eq!(result.sessions_cleaned, 0);
        assert_eq!(result.process_groups_signaled, 0);
    }

    #[test]
    fn cleanup_worktree_resources_on_nonexistent_path_returns_default() {
        // open_at fails, so no tmux commands run.
        let result = cleanup_worktree_resources(Path::new("/nonexistent/path/to/repo"))
            .expect("should succeed on missing path");

        assert!(!result.worktree_deleted);
        assert!(result.branch_deleted.is_none());
        assert_eq!(result.windows_closed, 0);
        assert_eq!(result.sessions_cleaned, 0);
        assert_eq!(result.process_groups_signaled, 0);
    }
}
