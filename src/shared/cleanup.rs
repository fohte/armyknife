//! Shared cleanup logic for agent sessions, notifications, and git worktrees.
//!
//! Both `agent watch` (session cleanup) and `agent close`/`agent clean` (worktree cleanup)
//! need to clean up related resources. This module provides the shared logic to
//! ensure consistent cleanup regardless of the entry point.

use std::collections::HashSet;
use std::path::Path;

use crate::commands::agent::store;
use crate::commands::agent::types::{Engine, Session, SessionStatus};
use crate::infra::git::GitRepo;
use crate::infra::process;
use crate::infra::tmux;
use crate::shared::worktree::{
    delete_branch_if_exists, delete_worktree, find_worktree_name, get_main_repo,
    get_worktree_branch,
};

/// Result of worktree resource cleanup.
#[derive(Debug, Default)]
pub struct WorktreeCleanupResult {
    /// Whether a git worktree was deleted.
    pub worktree_deleted: bool,
    /// Name of the branch that was deleted, if any.
    pub branch_deleted: Option<String>,
    /// Number of tmux windows that were closed.
    pub windows_closed: usize,
    /// Number of agent sessions cleaned up.
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
/// worktree itself, branch, tmux windows, agent session files, and notifications.
///
/// `cwd` can be any path inside the worktree (including subdirectories);
/// the worktree root is resolved via `repo.workdir()`.
///
/// If `cwd` is not inside a git worktree, returns a default (no-op) result.
pub fn cleanup_worktree_resources(cwd: &Path) -> anyhow::Result<WorktreeCleanupResult> {
    // The session-list TUI does not resolve merge status before deleting a
    // worktree, so this path intentionally does not invoke post-delete hooks.
    cleanup_worktree_resources_with_post_delete(cwd, || {})
}

/// Cleans up a worktree and invokes `after_delete` after git removes it but
/// before cleanup can terminate a pane running the caller.
pub fn cleanup_worktree_resources_with_post_delete(
    cwd: &Path,
    after_delete: impl FnOnce(),
) -> anyhow::Result<WorktreeCleanupResult> {
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

    let mut result =
        cleanup_worktree_by_name(&main_repo, &worktree_name, &worktree_root, after_delete)?;
    if result.worktree_deleted {
        result.worktree_root = Some(worktree_root);
    }
    Ok(result)
}

/// Cleans up a worktree and invokes `after_delete` once git confirms removal,
/// before session and tmux cleanup runs.
///
/// `worktree_path` is the filesystem path of the worktree root, used for
/// tmux window and session file lookup.
pub fn cleanup_worktree_by_name(
    repo: &GitRepo,
    worktree_name: &str,
    worktree_path: &Path,
    after_delete: impl FnOnce(),
) -> anyhow::Result<WorktreeCleanupResult> {
    // Collect tmux window IDs and process groups rooted in the worktree
    // before deleting it (the path must still exist to match against).
    let path_str = worktree_path.to_string_lossy();
    let window_ids = tmux::get_window_ids_in_path(&path_str);
    let orphan_pgids = process::find_pgids_in_path(worktree_path);

    let mut result = delete_worktree_and_branch(repo, worktree_name);

    // Clean up sessions BEFORE killing tmux windows: if the caller is running
    // inside one of those windows (e.g. `a agent close` invoked from the
    // worktree's own pane), kill_window terminates the caller's pane and
    // SIGHUPs this very process, leaving Paused sessions orphaned on disk.
    run_post_delete_cleanup(result.worktree_deleted, after_delete, || {
        result.sessions_cleaned = cleanup_sessions_in_path(worktree_path).unwrap_or(0);
        result.process_groups_signaled = process::kill_process_groups(&orphan_pgids);

        for window_id in &window_ids {
            if tmux::kill_window(window_id).is_ok() {
                result.windows_closed += 1;
            }
        }
    });

    Ok(result)
}

fn run_post_delete_cleanup(
    worktree_deleted: bool,
    after_delete: impl FnOnce(),
    cleanup: impl FnOnce(),
) {
    if worktree_deleted {
        after_delete();
        cleanup();
    }
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

/// Returns true if the pane hosting this session still runs a live agent
/// process that needs SIGTERM before we drop the session file.
///
/// Paused sessions were already SIGTERM'd by `agent sweep`, and Ended sessions
/// exited on their own. In both cases the pane's foreground process is the
/// user's shell (possibly the shell that just invoked `a agent close` from
/// within the worktree). Signaling that shell would kill the very process we
/// are running in, so restrict SIGTERM to statuses where a Claude process is
/// still expected to be alive.
fn should_sigterm_session(status: SessionStatus) -> bool {
    match status {
        SessionStatus::Running | SessionStatus::WaitingInput | SessionStatus::Stopped => true,
        SessionStatus::Paused | SessionStatus::Ended => false,
    }
}

/// Cleans up active agent session files and notification groups for sessions
/// whose `cwd` is inside `worktree_path`.
///
/// Every stored status is considered so `Ended` sessions with stale notifications
/// are included. Ended session records are retained for later delegated-session lookup.
///
/// For each matching session:
/// 1. Archives Codex threads, except the current thread, which is archived
///    after this cleanup process exits.
/// 2. If the session's Claude process is still expected to be alive and the
///    tmux pane is alive, sends SIGTERM to it
/// 3. Deletes the session file unless the session has already ended
/// 4. Removes the notification group on a best-effort basis
///
/// Returns the number of sessions cleaned up.
pub fn cleanup_sessions_in_path(worktree_path: &Path) -> anyhow::Result<usize> {
    let sessions = store::list_all_sessions()?;
    let alive_panes = tmux::list_all_pane_ids().unwrap_or_default();
    let own_codex_session_id = crate::shared::env_var::EnvVars::load().codex_session_id;

    Ok(cleanup_sessions_with_deferred_archive(
        &sessions,
        SessionCleanupContext {
            worktree_path,
            alive_panes: &alive_panes,
            own_codex_session_id: own_codex_session_id.as_deref(),
        },
        SessionCleanupOperations {
            send_sigterm: tmux::send_sigterm_to_pane,
            delete_session: store::delete_session_without_archive,
            remove_notification_group: crate::infra::notification::remove_group,
            archive_codex_thread: crate::commands::agent::codex_steer::archive_thread,
            defer_archive: crate::commands::agent::spawn_after_parent_exit,
        },
    ))
}

struct SessionCleanupOperations<
    SendSigterm,
    DeleteSession,
    RemoveNotification,
    Archive,
    DeferArchive,
> {
    send_sigterm: SendSigterm,
    delete_session: DeleteSession,
    remove_notification_group: RemoveNotification,
    archive_codex_thread: Archive,
    defer_archive: DeferArchive,
}

fn cleanup_sessions_with_deferred_archive<
    SendSigterm,
    DeleteSession,
    RemoveNotification,
    Archive,
    DeferArchive,
>(
    sessions: &[Session],
    context: SessionCleanupContext<'_>,
    operations: SessionCleanupOperations<
        SendSigterm,
        DeleteSession,
        RemoveNotification,
        Archive,
        DeferArchive,
    >,
) -> usize
where
    SendSigterm: FnMut(&str),
    DeleteSession: FnMut(&str) -> anyhow::Result<()>,
    RemoveNotification: FnMut(&str) -> anyhow::Result<()>,
    Archive: FnMut(&str) -> anyhow::Result<()>,
    DeferArchive: FnMut(&str) -> anyhow::Result<()>,
{
    let deferred_archive = context
        .own_codex_session_id
        .filter(|session_id| {
            sessions.iter().any(|session| {
                session.engine == Engine::Codex
                    && session.session_id == *session_id
                    && session.cwd.starts_with(context.worktree_path)
            })
        })
        .map(str::to_string);

    let SessionCleanupOperations {
        send_sigterm,
        delete_session,
        remove_notification_group,
        archive_codex_thread,
        mut defer_archive,
    } = operations;

    let cleaned = cleanup_sessions(
        sessions,
        context,
        send_sigterm,
        delete_session,
        remove_notification_group,
        archive_codex_thread,
    );

    if let Some(session_id) = deferred_archive
        && let Err(error) = defer_archive(&session_id)
    {
        eprintln!(
            "Warning: Failed to defer Codex thread archive for session {session_id}: {error:#}"
        );
        tracing::warn!(
            target: "armyknife::shared::cleanup",
            event = "cleanup.codex_archive.defer_err",
            session = %session_id,
            msg = format!("failed to defer Codex thread archive: {error:#}"),
        );
    }

    cleaned
}

struct SessionCleanupContext<'a> {
    worktree_path: &'a Path,
    alive_panes: &'a HashSet<String>,
    own_codex_session_id: Option<&'a str>,
}

fn cleanup_sessions(
    sessions: &[Session],
    context: SessionCleanupContext<'_>,
    mut send_sigterm: impl FnMut(&str),
    mut delete_session: impl FnMut(&str) -> anyhow::Result<()>,
    mut remove_notification_group: impl FnMut(&str) -> anyhow::Result<()>,
    mut archive_codex_thread: impl FnMut(&str) -> anyhow::Result<()>,
) -> usize {
    let mut cleaned = 0;

    for session in sessions {
        if session.cwd.starts_with(context.worktree_path) {
            let is_own_codex_session = session.engine == Engine::Codex
                && context.own_codex_session_id == Some(session.session_id.as_str());

            if session.engine == Engine::Codex && !is_own_codex_session {
                store::archive_codex_thread_before_delete_with(
                    session,
                    &mut archive_codex_thread,
                    store::ArchiveLogContext::WorktreeCleanup,
                );
            }

            if !is_own_codex_session
                && should_sigterm_session(session.status)
                && let Some(ref tmux_info) = session.tmux_info
                && context.alive_panes.contains(&tmux_info.pane_id)
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
    use crate::commands::agent::types::Engine;
    use crate::shared::testing::TestRepo;
    use chrono::Utc;
    use rstest::rstest;
    use std::cell::RefCell;
    use std::path::PathBuf;

    fn make_session(session_id: &str, cwd: &Path, engine: Engine) -> Session {
        Session {
            session_id: session_id.to_string(),
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: cwd.to_path_buf(),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Stopped,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
            label: None,
            work_type: None,
            work_type_pinned: false,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: Default::default(),
            pending_agent_task_ids: Default::default(),
            pending_permission_agent_ids: Default::default(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            engine,
        }
    }

    #[rstest]
    fn defers_current_codex_archive_until_cleanup_finishes() {
        let worktree_path = PathBuf::from("/tmp/example-worktree");
        let sessions = vec![
            make_session("current-session", &worktree_path, Engine::Codex),
            make_session("other-session", &worktree_path, Engine::Codex),
        ];
        let alive_panes = HashSet::new();
        let events = RefCell::new(Vec::new());

        let cleaned = cleanup_sessions_with_deferred_archive(
            &sessions,
            SessionCleanupContext {
                worktree_path: &worktree_path,
                alive_panes: &alive_panes,
                own_codex_session_id: Some("current-session"),
            },
            SessionCleanupOperations {
                send_sigterm: |_pane_id: &str| {},
                delete_session: |session_id: &str| {
                    events.borrow_mut().push(format!("delete:{session_id}"));
                    Ok(())
                },
                remove_notification_group: |session_id: &str| {
                    events
                        .borrow_mut()
                        .push(format!("notification:{session_id}"));
                    Ok(())
                },
                archive_codex_thread: |session_id: &str| {
                    events.borrow_mut().push(format!("archive:{session_id}"));
                    Ok(())
                },
                defer_archive: |session_id: &str| {
                    events.borrow_mut().push(format!("defer:{session_id}"));
                    Ok(())
                },
            },
        );

        assert_eq!(
            (cleaned, events.into_inner()),
            (
                2,
                vec![
                    "delete:current-session".to_string(),
                    "notification:current-session".to_string(),
                    "archive:other-session".to_string(),
                    "delete:other-session".to_string(),
                    "notification:other-session".to_string(),
                    "defer:current-session".to_string(),
                ],
            ),
        );
    }

    #[rstest]
    #[case::deleted(true, vec!["hook", "cleanup"])]
    #[case::not_deleted(false, vec![])]
    fn post_delete_cleanup_runs_only_after_successful_deletion(
        #[case] worktree_deleted: bool,
        #[case] expected: Vec<&str>,
    ) {
        let events = RefCell::new(Vec::new());

        run_post_delete_cleanup(
            worktree_deleted,
            || events.borrow_mut().push("hook"),
            || events.borrow_mut().push("cleanup"),
        );

        assert_eq!(events.into_inner(), expected);
    }

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
        // when `a agent close` runs from that same pane.
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
