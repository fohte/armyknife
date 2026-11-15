use std::path::{Path, PathBuf};

use crate::infra::git::GitRepo;
use crate::infra::process::{self, ProcessInPath};
use crate::infra::tmux;
use crate::shared::worktree::{
    delete_branch_if_exists, delete_worktree, delete_worktree_if_clean, find_worktree_name,
    get_main_repo, get_worktree_branch,
};

#[derive(Debug, thiserror::Error)]
pub enum WorktreeProcessGuardError {
    #[error(
        "Worktree '{worktree_path}' contains non-shell processes: {process_list}; stop them or pass --force to delete the worktree"
    )]
    ActiveProcesses {
        worktree_path: PathBuf,
        process_list: String,
    },
    #[error(
        "Could not inspect processes in worktree '{worktree_path}'; ensure lsof and ps are available and working, or pass --force to delete the worktree"
    )]
    InspectionUnavailable { worktree_path: PathBuf },
}

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
    pub worktree_root: Option<PathBuf>,
}

/// Cleans up all resources associated with a worktree at `cwd`:
/// worktree itself, branch, tmux windows, agent session records, and notifications.
///
/// `cwd` can be any path inside the worktree (including subdirectories);
/// the worktree root is resolved via `repo.workdir()`.
///
/// If `cwd` is not inside a git worktree, returns a default (no-op) result.
pub fn cleanup_worktree_resources(
    cwd: &Path,
    force: bool,
) -> anyhow::Result<WorktreeCleanupResult> {
    // The session-list TUI does not resolve merge status before deleting a
    // worktree, so this path intentionally does not invoke post-delete hooks.
    cleanup_worktree_resources_with_post_delete(cwd, force, || {})
}

/// Cleans up a worktree and invokes `after_delete` after git removes it but
/// before cleanup can terminate a pane running the caller.
pub fn cleanup_worktree_resources_with_post_delete(
    cwd: &Path,
    force: bool,
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

    let mut result = cleanup_worktree_by_name(
        &main_repo,
        &worktree_name,
        &worktree_root,
        force,
        after_delete,
    )?;
    if result.worktree_deleted {
        result.worktree_root = Some(worktree_root);
    }
    Ok(result)
}

/// Cleans up a worktree and invokes `after_delete` once git confirms removal,
/// before session and tmux cleanup runs.
///
/// `worktree_path` is the filesystem path of the worktree root, used for
/// tmux window and session record lookup.
pub fn cleanup_worktree_by_name(
    repo: &GitRepo,
    worktree_name: &str,
    worktree_path: &Path,
    force: bool,
    after_delete: impl FnOnce(),
) -> anyhow::Result<WorktreeCleanupResult> {
    cleanup_worktree_by_name_with_operations(
        worktree_path,
        force,
        after_delete,
        WorktreeCleanupOperations {
            detect_processes: process::find_non_shell_processes_in_path,
            find_windows: |path: &Path| tmux::get_window_ids_in_path(&path.to_string_lossy()),
            find_groups: process::find_pgids_in_path,
            clean_sessions: |path: &Path, signal_agents: bool| {
                super::cleanup_sessions_in_path_with_policy(path, signal_agents).unwrap_or(0)
            },
            kill_groups: process::kill_process_groups,
            kill_window: |window_id: &str| tmux::kill_window(window_id).is_ok(),
            delete_worktree: |force, path: PathBuf, detect_processes| {
                delete_worktree_and_branch(repo, worktree_name, &path, force, || {
                    check_process_guard(&path, force, detect_processes)
                })
            },
        },
    )
}

struct WorktreeCleanupOperations<
    DetectProcesses,
    FindWindows,
    FindGroups,
    CleanSessions,
    KillGroups,
    KillWindow,
    DeleteWorktree,
> {
    detect_processes: DetectProcesses,
    find_windows: FindWindows,
    find_groups: FindGroups,
    clean_sessions: CleanSessions,
    kill_groups: KillGroups,
    kill_window: KillWindow,
    delete_worktree: DeleteWorktree,
}

fn cleanup_worktree_by_name_with_operations<
    DetectProcesses,
    FindWindows,
    FindGroups,
    CleanSessions,
    KillGroups,
    KillWindow,
    DeleteWorktree,
>(
    worktree_path: &Path,
    force: bool,
    after_delete: impl FnOnce(),
    operations: WorktreeCleanupOperations<
        DetectProcesses,
        FindWindows,
        FindGroups,
        CleanSessions,
        KillGroups,
        KillWindow,
        DeleteWorktree,
    >,
) -> anyhow::Result<WorktreeCleanupResult>
where
    DetectProcesses: FnOnce(&Path) -> Option<Vec<ProcessInPath>>,
    FindWindows: FnOnce(&Path) -> Vec<String>,
    FindGroups: FnOnce(&Path) -> Vec<libc::pid_t>,
    CleanSessions: FnOnce(&Path, bool) -> usize,
    KillGroups: FnOnce(&[libc::pid_t]) -> usize,
    KillWindow: FnMut(&str) -> bool,
    DeleteWorktree: FnOnce(bool, PathBuf, DetectProcesses) -> anyhow::Result<WorktreeCleanupResult>,
{
    let WorktreeCleanupOperations {
        detect_processes,
        find_windows,
        find_groups,
        clean_sessions,
        kill_groups,
        mut kill_window,
        delete_worktree,
    } = operations;

    // Collect tmux window IDs before deleting because the path is needed for matching.
    let window_ids = find_windows(worktree_path);
    let orphan_pgids = if force {
        find_groups(worktree_path)
    } else {
        Vec::new()
    };

    let mut result = delete_worktree(force, worktree_path.to_path_buf(), detect_processes)?;

    // Clean up sessions before killing panes so a caller in the window can finish.
    run_post_delete_cleanup(result.worktree_deleted, after_delete, || {
        result.sessions_cleaned = clean_sessions(worktree_path, force);
        result.process_groups_signaled = if force { kill_groups(&orphan_pgids) } else { 0 };

        for window_id in &window_ids {
            if kill_window(window_id) {
                result.windows_closed += 1;
            }
        }
    });

    Ok(result)
}

fn check_process_guard(
    worktree_path: &Path,
    force: bool,
    detect_processes: impl FnOnce(&Path) -> Option<Vec<ProcessInPath>>,
) -> anyhow::Result<()> {
    if force {
        return Ok(());
    }
    let processes = detect_processes(worktree_path).ok_or_else(|| {
        WorktreeProcessGuardError::InspectionUnavailable {
            worktree_path: worktree_path.to_path_buf(),
        }
    })?;
    if !processes.is_empty() {
        let process_list = processes
            .iter()
            .map(|process| format!("{} ({})", process.pid, process.comm))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(WorktreeProcessGuardError::ActiveProcesses {
            worktree_path: worktree_path.to_path_buf(),
            process_list,
        }
        .into());
    }
    Ok(())
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

fn delete_worktree_and_branch(
    repo: &GitRepo,
    worktree_name: &str,
    worktree_path: &Path,
    force: bool,
    before_delete: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<WorktreeCleanupResult> {
    let branch_name = get_worktree_branch(repo, worktree_name);

    let worktree_deleted = delete_worktree_if_clean(worktree_path, force, before_delete, || {
        Ok(delete_worktree(repo, worktree_name).unwrap_or(false))
    })?;

    let branch_deleted = if worktree_deleted {
        branch_name.filter(|branch| delete_branch_if_exists(repo, branch))
    } else {
        None
    };

    Ok(WorktreeCleanupResult {
        worktree_deleted,
        branch_deleted,
        ..WorktreeCleanupResult::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::testing::TestRepo;
    use rstest::rstest;
    use std::cell::RefCell;
    use tempfile::TempDir;

    #[test]
    fn delete_worktree_and_branch_deletes_both() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("cleanup-test");

        let repo = test_repo.open();
        let result = delete_worktree_and_branch(
            &repo,
            "cleanup-test",
            &test_repo.worktree_path("cleanup-test"),
            false,
            || Ok(()),
        )
        .expect("clean worktree should be deleted");

        assert!(result.worktree_deleted);
        assert_eq!(result.branch_deleted, Some("cleanup-test".to_string()));
        assert_eq!(result.windows_closed, 0);
        assert_eq!(result.sessions_cleaned, 0);
        assert_eq!(result.process_groups_signaled, 0);

        let list =
            crate::infra::git::cmd::run_git(repo.workdir(), ["worktree", "list", "--porcelain"])
                .unwrap();
        assert!(!list.contains("/.worktrees/cleanup-test"));
    }

    #[rstest]
    #[case::active_process_without_force(
        false,
        Some(vec![ProcessInPath { pid: 42, comm: "editor".to_string() }]),
        (false, Some("active"), true, vec!["windows", "detect"]),
    )]
    #[case::inspection_unavailable_without_force(
        false,
        None,
        (false, Some("unavailable"), true, vec!["windows", "detect"]),
    )]
    #[case::shell_only_without_force(
        false,
        Some(vec![]),
        (true, None, false, vec!["windows", "detect", "delete:false", "after_delete", "sessions:false", "window_closed"]),
    )]
    #[case::force_skips_the_guard_and_signals_process_groups(
        true,
        None,
        (true, None, false, vec!["windows", "groups_detect", "delete:true", "after_delete", "sessions:true", "groups:[77]", "window_closed"]),
    )]
    fn cleanup_checks_processes_before_deleting_and_signals_only_with_force(
        #[case] force: bool,
        #[case] detected_processes: Option<Vec<ProcessInPath>>,
        #[case] expected: (bool, Option<&'static str>, bool, Vec<&'static str>),
    ) {
        let temp_dir = TempDir::new().expect("temporary worktree directory should be created");
        let worktree_path = temp_dir.path().join("worktree");
        std::fs::create_dir(&worktree_path).expect("worktree directory should be created");
        let events = RefCell::new(Vec::new());

        let result = cleanup_worktree_by_name_with_operations(
            &worktree_path,
            force,
            || events.borrow_mut().push("after_delete"),
            WorktreeCleanupOperations {
                detect_processes: |_: &Path| {
                    events.borrow_mut().push("detect");
                    detected_processes
                },
                find_windows: |_: &Path| {
                    events.borrow_mut().push("windows");
                    vec!["@process-guard-test".to_string()]
                },
                find_groups: |_: &Path| {
                    events.borrow_mut().push("groups_detect");
                    vec![77]
                },
                clean_sessions: |_: &Path, signal_agents: bool| {
                    events.borrow_mut().push(if signal_agents {
                        "sessions:true"
                    } else {
                        "sessions:false"
                    });
                    0
                },
                kill_groups: |pgids: &[libc::pid_t]| {
                    events.borrow_mut().push(if pgids.is_empty() {
                        "groups:[]"
                    } else {
                        "groups:[77]"
                    });
                    pgids.len()
                },
                kill_window: |_: &str| {
                    events.borrow_mut().push("window_closed");
                    true
                },
                delete_worktree: |force, path: PathBuf, detect_processes| {
                    check_process_guard(&path, force, detect_processes)?;
                    events
                        .borrow_mut()
                        .push(if force { "delete:true" } else { "delete:false" });
                    std::fs::remove_dir_all(path).expect("worktree directory should be deleted");
                    Ok(WorktreeCleanupResult {
                        worktree_deleted: true,
                        ..WorktreeCleanupResult::default()
                    })
                },
            },
        );

        let guard_error = result.as_ref().err().and_then(|error| {
            error
                .downcast_ref::<WorktreeProcessGuardError>()
                .map(|error| match error {
                    WorktreeProcessGuardError::ActiveProcesses { .. } => "active",
                    WorktreeProcessGuardError::InspectionUnavailable { .. } => "unavailable",
                })
        });
        assert_eq!(
            (
                result.as_ref().is_ok_and(|result| result.worktree_deleted),
                guard_error,
                worktree_path.is_dir(),
                events.into_inner(),
            ),
            expected,
        );
    }

    #[rstest]
    #[case::untracked_without_force(false, false, (false, true, true, true))]
    #[case::untracked_with_force(false, true, (true, false, false, false))]
    #[case::tracked_without_force(true, false, (false, true, true, true))]
    #[case::tracked_with_force(true, true, (true, false, false, false))]
    fn deletion_checks_real_git_status_before_removing_dirty_worktrees(
        #[case] tracked: bool,
        #[case] force: bool,
        #[case] expected: (bool, bool, bool, bool),
    ) {
        let test_repo = TestRepo::new();
        if tracked {
            test_repo.commit_file("tracked.txt", "original");
        }
        test_repo.create_worktree("dirty-worktree");

        let worktree_path = test_repo.worktree_path("dirty-worktree");
        let dirty_file = if tracked {
            worktree_path.join("tracked.txt")
        } else {
            worktree_path.join("untracked.txt")
        };
        std::fs::write(&dirty_file, "changed").expect("write dirty worktree file");

        let repo = test_repo.open();
        if !tracked {
            crate::infra::git::cmd::run_git(
                repo.workdir(),
                ["config", "status.showUntrackedFiles", "no"],
            )
            .expect("configure git to hide untracked files by default");
        }

        let result =
            delete_worktree_and_branch(&repo, "dirty-worktree", &worktree_path, force, || Ok(()));
        let observed = (
            result.as_ref().is_ok_and(|result| result.worktree_deleted),
            result.as_ref().err().is_some_and(|error| {
                error
                    .downcast_ref::<crate::shared::worktree::DirtyWorktreeError>()
                    .is_some()
            }),
            worktree_path.is_dir(),
            crate::shared::worktree::list_linked_worktrees(&repo)
                .expect("list worktrees")
                .iter()
                .any(|worktree| worktree.name == "dirty-worktree"),
        );

        assert_eq!(observed, expected);
    }

    #[test]
    fn delete_worktree_and_branch_returns_false_for_nonexistent() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        let result =
            delete_worktree_and_branch(&repo, "nonexistent", &test_repo.path(), false, || Ok(()))
                .expect("missing worktree should return false");

        assert!(!result.worktree_deleted);
        assert!(result.branch_deleted.is_none());
    }

    #[test]
    fn cleanup_worktree_resources_on_non_worktree_returns_default() {
        let test_repo = TestRepo::new();
        let result = cleanup_worktree_resources(&test_repo.path(), false)
            .expect("should succeed on main repo");

        assert!(!result.worktree_deleted);
        assert!(result.branch_deleted.is_none());
        assert_eq!(result.windows_closed, 0);
        assert_eq!(result.sessions_cleaned, 0);
        assert_eq!(result.process_groups_signaled, 0);
    }

    #[test]
    fn cleanup_worktree_resources_on_nonexistent_path_returns_default() {
        let result = cleanup_worktree_resources(Path::new("/nonexistent/path/to/repo"), false)
            .expect("should succeed on missing path");

        assert!(!result.worktree_deleted);
        assert!(result.branch_deleted.is_none());
        assert_eq!(result.windows_closed, 0);
        assert_eq!(result.sessions_cleaned, 0);
        assert_eq!(result.process_groups_signaled, 0);
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
}
