use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::infra::git::GitRepo;
use crate::infra::process::{self, ProcessInPath, WorktreeProcessInspection};
use crate::infra::tmux;
use crate::shared::worktree::{
    delete_branch_if_exists, delete_worktree, delete_worktree_if_clean, find_worktree_name,
    get_main_repo, get_worktree_branch,
};

type BeforeWorktreeDelete<'a> = Box<dyn FnOnce() -> anyhow::Result<()> + 'a>;

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
        "Could not inspect processes in worktree '{worktree_path}'; pass --force to delete the worktree"
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
    /// Number of tmux panes that were closed.
    pub panes_closed: usize,
    /// Number of agent sessions cleaned up.
    pub sessions_cleaned: usize,
    /// Number of process groups signaled after deletion.
    pub process_groups_signaled: usize,
    /// The resolved worktree root path (set when worktree_deleted is true).
    /// Use this instead of raw cwd for path matching, since cwd may be a
    /// subdirectory.
    pub worktree_root: Option<PathBuf>,
}

/// Cleans up all resources associated with a worktree at `cwd`:
/// worktree itself, branch, tmux panes, agent session records, and notifications.
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
/// tmux pane and session record lookup.
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
            inspect_processes: process::inspect_worktree_processes,
            find_panes: |path: &Path| tmux::get_pane_ids_in_path(&path.to_string_lossy()),
            find_groups: process::find_pgids_in_path,
            clean_sessions: |path: &Path, signal_agents: bool| {
                super::cleanup_sessions_in_path_with_policy(path, signal_agents).unwrap_or(0)
            },
            kill_groups: process::kill_process_groups,
            kill_pane: |pane_id: &str| tmux::kill_pane(pane_id).is_ok(),
            delete_worktree: |force, path: PathBuf, before_delete| {
                delete_worktree_and_branch(repo, worktree_name, &path, force, before_delete)
            },
        },
    )
}

struct WorktreeCleanupOperations<
    InspectProcesses,
    FindPanes,
    FindGroups,
    CleanSessions,
    KillGroups,
    KillPane,
    DeleteWorktree,
> {
    inspect_processes: InspectProcesses,
    find_panes: FindPanes,
    find_groups: FindGroups,
    clean_sessions: CleanSessions,
    kill_groups: KillGroups,
    kill_pane: KillPane,
    delete_worktree: DeleteWorktree,
}

fn cleanup_worktree_by_name_with_operations<
    'a,
    InspectProcesses,
    FindPanes,
    FindGroups,
    CleanSessions,
    KillGroups,
    KillPane,
    DeleteWorktree,
>(
    worktree_path: &Path,
    force: bool,
    after_delete: impl FnOnce(),
    operations: WorktreeCleanupOperations<
        InspectProcesses,
        FindPanes,
        FindGroups,
        CleanSessions,
        KillGroups,
        KillPane,
        DeleteWorktree,
    >,
) -> anyhow::Result<WorktreeCleanupResult>
where
    InspectProcesses: FnOnce(&Path) -> Option<WorktreeProcessInspection> + 'a,
    FindPanes: FnOnce(&Path) -> Vec<String>,
    FindGroups: FnOnce(&Path) -> Vec<libc::pid_t>,
    CleanSessions: FnOnce(&Path, bool) -> usize,
    KillGroups: FnOnce(&[libc::pid_t]) -> usize,
    KillPane: FnMut(&str) -> bool,
    DeleteWorktree:
        FnOnce(bool, PathBuf, BeforeWorktreeDelete<'a>) -> anyhow::Result<WorktreeCleanupResult>,
{
    let WorktreeCleanupOperations {
        inspect_processes,
        find_panes,
        find_groups,
        clean_sessions,
        kill_groups,
        mut kill_pane,
        delete_worktree,
    } = operations;

    // Collect tmux pane IDs before deleting because the path is needed for matching.
    let pane_ids = find_panes(worktree_path);
    let orphan_pgids = if force {
        find_groups(worktree_path)
    } else {
        Vec::new()
    };

    let guard_path = worktree_path.to_path_buf();
    let daemon_descendant_pgids = Rc::new(RefCell::new(Vec::new()));
    let guard_pgids = Rc::clone(&daemon_descendant_pgids);
    let before_delete: BeforeWorktreeDelete = Box::new(move || {
        check_process_guard(&guard_path, force, |path| {
            let inspection = inspect_processes(path)?;
            *guard_pgids.borrow_mut() = inspection.daemon_descendant_pgids;
            Some(inspection.non_shell_processes)
        })
    });
    let mut result = delete_worktree(force, worktree_path.to_path_buf(), before_delete)?;
    let daemon_descendant_pgids = daemon_descendant_pgids.borrow().clone();

    // Clean up sessions before killing panes so a caller in the pane can finish.
    run_post_delete_cleanup(result.worktree_deleted, after_delete, || {
        result.sessions_cleaned = clean_sessions(worktree_path, force);
        let process_groups = if force {
            &orphan_pgids
        } else {
            &daemon_descendant_pgids
        };
        result.process_groups_signaled = if process_groups.is_empty() {
            0
        } else {
            kill_groups(process_groups)
        };
        result.panes_closed = kill_worktree_panes(
            &pane_ids,
            tmux::current_pane_id_from_env().as_deref(),
            |pane_id| kill_pane(pane_id),
        );
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

fn kill_worktree_panes(
    pane_ids: &[String],
    caller_pane_id: Option<&str>,
    mut kill_pane: impl FnMut(&str) -> bool,
) -> usize {
    let caller_pane_id = caller_pane_id.and_then(|caller_pane_id| {
        pane_ids
            .iter()
            .any(|pane_id| pane_id == caller_pane_id)
            .then_some(caller_pane_id)
    });

    let mut panes_closed = 0;
    for pane_id in pane_ids
        .iter()
        .filter(|pane_id| Some(pane_id.as_str()) != caller_pane_id)
    {
        if kill_pane(pane_id) {
            panes_closed += 1;
        }
    }
    if let Some(caller_pane_id) = caller_pane_id
        && kill_pane(caller_pane_id)
    {
        panes_closed += 1;
    }
    panes_closed
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
    use rstest::{fixture, rstest};
    use std::cell::RefCell;
    use tempfile::TempDir;

    struct CleanupTestContext {
        _temp_dir: TempDir,
        worktree_path: PathBuf,
        events: RefCell<Vec<&'static str>>,
    }

    #[fixture]
    fn cleanup_test_context() -> CleanupTestContext {
        let temp_dir = TempDir::new().expect("temporary worktree directory should be created");
        let worktree_path = temp_dir.path().join("worktree");
        std::fs::create_dir(&worktree_path).expect("worktree directory should be created");
        CleanupTestContext {
            _temp_dir: temp_dir,
            worktree_path,
            events: RefCell::new(Vec::new()),
        }
    }

    #[test]
    fn closes_caller_pane_last_and_counts_successes() {
        let pane_ids = vec!["%pane-a".to_string(), "%pane-b".to_string()];
        let closed_panes = RefCell::new(Vec::new());

        let panes_closed = kill_worktree_panes(&pane_ids, Some("%pane-a"), |pane_id| {
            closed_panes.borrow_mut().push(pane_id.to_string());
            pane_id == "%pane-b"
        });

        assert_eq!(
            (closed_panes.into_inner(), panes_closed),
            (vec!["%pane-b".to_string(), "%pane-a".to_string()], 1),
        );
    }

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
        assert_eq!(result.panes_closed, 0);
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
        Some(WorktreeProcessInspection {
            non_shell_processes: vec![ProcessInPath { pid: 42, comm: "editor".to_string() }],
            daemon_descendant_pgids: vec![],
        }),
        (false, Some("active"), true, 0, vec!["panes", "detect"]),
    )]
    #[case::inspection_unavailable_without_force(
        false,
        None,
        (false, Some("unavailable"), true, 0, vec!["panes", "detect"]),
    )]
    #[case::shell_only_without_force(
        false,
        Some(WorktreeProcessInspection::default()),
        (true, None, false, 0, vec!["panes", "detect", "delete:false", "after_delete", "sessions:false", "pane_closed"]),
    )]
    #[case::daemon_groups_are_signaled_after_delete(
        false,
        Some(WorktreeProcessInspection {
            non_shell_processes: vec![],
            daemon_descendant_pgids: vec![88],
        }),
        (true, None, false, 1, vec!["panes", "detect", "delete:false", "after_delete", "sessions:false", "groups:[88]", "pane_closed"]),
    )]
    #[case::force_skips_the_guard_and_signals_process_groups(
        true,
        None,
        (true, None, false, 1, vec!["panes", "groups_detect", "delete:true", "after_delete", "sessions:true", "groups:[77]", "pane_closed"]),
    )]
    fn cleanup_checks_processes_before_deleting_and_signals_after_success(
        cleanup_test_context: CleanupTestContext,
        #[case] force: bool,
        #[case] detected_processes: Option<WorktreeProcessInspection>,
        #[case] expected: (bool, Option<&'static str>, bool, usize, Vec<&'static str>),
    ) {
        let CleanupTestContext {
            worktree_path,
            events,
            ..
        } = cleanup_test_context;

        let result = cleanup_worktree_by_name_with_operations(
            &worktree_path,
            force,
            || events.borrow_mut().push("after_delete"),
            WorktreeCleanupOperations {
                inspect_processes: |_: &Path| {
                    events.borrow_mut().push("detect");
                    detected_processes
                },
                find_panes: |_: &Path| {
                    events.borrow_mut().push("panes");
                    vec!["%process-guard-test".to_string()]
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
                    events.borrow_mut().push(match pgids {
                        [] => "groups:[]",
                        [77] => "groups:[77]",
                        [88] => "groups:[88]",
                        _ => "groups:other",
                    });
                    pgids.len()
                },
                kill_pane: |_: &str| {
                    events.borrow_mut().push("pane_closed");
                    true
                },
                delete_worktree: |force, path: PathBuf, before_delete: BeforeWorktreeDelete<'_>| {
                    before_delete()?;
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
                result
                    .as_ref()
                    .map_or(0, |result| result.process_groups_signaled),
                events.borrow().clone(),
            ),
            expected,
        );
    }

    #[rstest]
    fn cleanup_does_not_signal_codex_groups_when_deletion_fails(
        cleanup_test_context: CleanupTestContext,
    ) {
        let CleanupTestContext {
            worktree_path,
            events,
            ..
        } = cleanup_test_context;

        let result = cleanup_worktree_by_name_with_operations(
            &worktree_path,
            false,
            || events.borrow_mut().push("after_delete"),
            WorktreeCleanupOperations {
                inspect_processes: |_: &Path| {
                    events.borrow_mut().push("detect");
                    Some(WorktreeProcessInspection {
                        non_shell_processes: vec![],
                        daemon_descendant_pgids: vec![88],
                    })
                },
                find_panes: |_: &Path| {
                    events.borrow_mut().push("panes");
                    vec![]
                },
                find_groups: |_: &Path| vec![],
                clean_sessions: |_: &Path, _: bool| {
                    events.borrow_mut().push("sessions");
                    0
                },
                kill_groups: |_: &[libc::pid_t]| {
                    events.borrow_mut().push("groups_signaled");
                    1
                },
                kill_pane: |_: &str| true,
                delete_worktree: |_, _, before_delete: BeforeWorktreeDelete<'_>| {
                    before_delete()?;
                    events.borrow_mut().push("delete_failed");
                    Err(anyhow::anyhow!("fixture deletion failure"))
                },
            },
        );

        assert_eq!(
            (result.is_err(), events.into_inner()),
            (true, vec!["panes", "detect", "delete_failed"]),
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
        assert_eq!(result.panes_closed, 0);
        assert_eq!(result.sessions_cleaned, 0);
        assert_eq!(result.process_groups_signaled, 0);
    }

    #[test]
    fn cleanup_worktree_resources_on_nonexistent_path_returns_default() {
        let result = cleanup_worktree_resources(Path::new("/nonexistent/path/to/repo"), false)
            .expect("should succeed on missing path");

        assert!(!result.worktree_deleted);
        assert!(result.branch_deleted.is_none());
        assert_eq!(result.panes_closed, 0);
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
