//! Common worktree operations shared by worktree cleanup commands.

use std::path::PathBuf;

use anyhow::Context;

use super::worktree_error::{Result, WmError};
use crate::infra::git::GitRepo;
use crate::infra::git::cmd::run_git;

mod lock;

#[derive(Debug, thiserror::Error)]
#[error(
    "Worktree contains uncommitted changes or untracked files: {worktree_path}; pass --force to delete it"
)]
pub struct DirtyWorktreeError {
    worktree_path: PathBuf,
}

#[derive(Debug, thiserror::Error)]
#[error(
    "Worktree is being deleted by another process: {worktree_path}; retry after deletion finishes"
)]
pub struct WorktreeDeletionInProgressError {
    worktree_path: PathBuf,
}

impl WorktreeDeletionInProgressError {
    fn new(worktree_path: PathBuf) -> Self {
        Self { worktree_path }
    }
}

fn worktree_lock_error(
    error: lock::WorktreeLockError,
    worktree_path: &std::path::Path,
) -> anyhow::Error {
    match error {
        lock::WorktreeLockError::Busy => {
            WorktreeDeletionInProgressError::new(worktree_path.to_path_buf()).into()
        }
        lock::WorktreeLockError::Io(error) => error,
    }
}

impl DirtyWorktreeError {
    pub(crate) fn new(worktree_path: PathBuf) -> Self {
        Self { worktree_path }
    }

    pub(crate) fn worktree_path(&self) -> &std::path::Path {
        &self.worktree_path
    }
}

/// Refuses deletion when a worktree contains changes that would be discarded.
pub fn ensure_clean_for_deletion(
    worktree_path: &std::path::Path,
    force: bool,
) -> anyhow::Result<()> {
    if force || !worktree_path.is_dir() {
        return Ok(());
    }
    let lock_dir = lock::default_lock_dir()?;
    ensure_clean_for_deletion_with(worktree_path, force, &lock_dir, status_from_git)
}

pub(crate) fn uncommitted_files(worktree_path: &std::path::Path) -> anyhow::Result<Vec<String>> {
    if !worktree_path.is_dir() {
        return Ok(Vec::new());
    }
    let lock_dir = lock::default_lock_dir()?;
    uncommitted_files_with_status(worktree_path, &lock_dir)
}

fn uncommitted_files_with_status(
    worktree_path: &std::path::Path,
    lock_dir: &std::path::Path,
) -> anyhow::Result<Vec<String>> {
    uncommitted_files_with(worktree_path, lock_dir, status_from_git)
}

fn status_from_git(worktree_path: &std::path::Path) -> anyhow::Result<String> {
    run_git(
        worktree_path,
        ["status", "--porcelain=v1", "-z", "--untracked-files=normal"],
    )
}

fn uncommitted_files_with(
    worktree_path: &std::path::Path,
    lock_dir: &std::path::Path,
    status: impl FnOnce(&std::path::Path) -> anyhow::Result<String>,
) -> anyhow::Result<Vec<String>> {
    worktree_changes_with(worktree_path, lock_dir, status).map(|changes| changes.files)
}

struct WorktreeChanges {
    files: Vec<String>,
    is_dirty: bool,
}

fn worktree_changes_with(
    worktree_path: &std::path::Path,
    lock_dir: &std::path::Path,
    status: impl FnOnce(&std::path::Path) -> anyhow::Result<String>,
) -> anyhow::Result<WorktreeChanges> {
    if !worktree_path.is_dir() {
        return Ok(WorktreeChanges {
            files: Vec::new(),
            is_dirty: false,
        });
    }
    let _lock = lock::try_lock_shared(lock_dir, worktree_path)
        .map_err(|error| worktree_lock_error(error, worktree_path))?;
    if !worktree_path.is_dir() {
        return Ok(WorktreeChanges {
            files: Vec::new(),
            is_dirty: false,
        });
    }
    status(worktree_path).map(|status| WorktreeChanges {
        is_dirty: !status.is_empty(),
        files: parse_uncommitted_files(&status),
    })
}

fn parse_uncommitted_files(status: &str) -> Vec<String> {
    let mut records = status.split('\0');
    let mut files = Vec::new();
    while let Some(record) = records.next() {
        if record.len() < 4 {
            continue;
        }
        let status_code = &record[..2];
        files.push(record[3..].to_string());
        if status_code.contains('R') || status_code.contains('C') {
            let _ = records.next();
        }
    }
    files
}

fn ensure_clean_for_deletion_with(
    worktree_path: &std::path::Path,
    force: bool,
    lock_dir: &std::path::Path,
    status: impl FnOnce(&std::path::Path) -> anyhow::Result<String>,
) -> anyhow::Result<()> {
    if force || !worktree_path.is_dir() {
        return Ok(());
    }

    if worktree_changes_with(worktree_path, lock_dir, status)?.is_dirty {
        return Err(DirtyWorktreeError::new(worktree_path.to_path_buf()).into());
    }

    Ok(())
}

pub(crate) fn delete_worktree_if_clean(
    worktree_path: &std::path::Path,
    force: bool,
    before_delete: impl FnOnce() -> anyhow::Result<()>,
    delete: impl FnOnce() -> anyhow::Result<bool>,
) -> anyhow::Result<bool> {
    let lock_dir = lock::default_lock_dir()?;
    delete_worktree_if_clean_with(
        worktree_path,
        force,
        &lock_dir,
        status_from_git,
        before_delete,
        delete,
        || {},
    )
}

fn delete_worktree_if_clean_with(
    worktree_path: &std::path::Path,
    force: bool,
    lock_dir: &std::path::Path,
    status: impl FnOnce(&std::path::Path) -> anyhow::Result<String>,
    before_delete: impl FnOnce() -> anyhow::Result<()>,
    delete: impl FnOnce() -> anyhow::Result<bool>,
    lock_waiting: impl FnOnce(),
) -> anyhow::Result<bool> {
    with_worktree_deletion_lock_with_wait(lock_dir, worktree_path, lock_waiting, || {
        if !force && worktree_path.is_dir() {
            let status = status(worktree_path)?;
            if !status.is_empty() {
                return Err(DirtyWorktreeError::new(worktree_path.to_path_buf()).into());
            }
        }

        before_delete()?;
        delete()
    })
}

/// Normalize a branch name to a worktree directory name.
/// - Removes the given branch_prefix
/// - Replaces slashes with dashes
pub fn branch_to_worktree_name(branch: &str, branch_prefix: &str) -> String {
    let name_no_prefix = branch.strip_prefix(branch_prefix).unwrap_or(branch);
    name_no_prefix.replace('/', "-")
}

/// Default branch prefix for tests.
#[cfg(test)]
pub const BRANCH_PREFIX: &str = "fohte/";

/// Get the main repository, resolving from a worktree if necessary.
pub fn get_main_repo(repo: &GitRepo) -> Result<GitRepo> {
    repo.main_repo().map_err(|_| WmError::NotInGitRepo.into())
}

/// Get the branch name associated with a worktree.
pub fn get_worktree_branch(repo: &GitRepo, worktree_name: &str) -> Option<String> {
    get_worktree_branch_checked(repo, worktree_name)
        .ok()
        .flatten()
}

pub(crate) fn get_worktree_branch_checked(
    repo: &GitRepo,
    worktree_name: &str,
) -> anyhow::Result<Option<String>> {
    let worktrees = list_worktrees_raw(repo)?;
    let entry = worktrees
        .into_iter()
        .find(|worktree| worktree.name == worktree_name)
        .ok_or_else(|| anyhow::anyhow!("Worktree `{worktree_name}` was not found"))?;
    Ok(Some(entry.branch).filter(|branch| !branch.is_empty() && branch != "(detached)"))
}

/// Delete a worktree by name. Returns true if successful.
pub fn delete_worktree(repo: &GitRepo, worktree_name: &str) -> Result<bool> {
    let Some(worktree_path) = worktree_path_for_name(repo, worktree_name) else {
        return Ok(false);
    };

    let lock_dir = lock::default_lock_dir()?;
    with_worktree_deletion_lock(&lock_dir, &worktree_path, || {
        delete_worktree_at_path(repo, worktree_name, &worktree_path)
    })
}

pub(crate) fn delete_worktree_after_lock(
    repo: &GitRepo,
    worktree_name: &str,
) -> anyhow::Result<bool> {
    let Some(worktree_path) = worktree_path_for_name(repo, worktree_name) else {
        return Ok(false);
    };
    delete_worktree_at_path(repo, worktree_name, &worktree_path)
}

fn worktree_path_for_name(repo: &GitRepo, worktree_name: &str) -> Option<PathBuf> {
    match list_worktrees_raw(repo) {
        Ok(list) => match list
            .into_iter()
            .find(|worktree| worktree.name == worktree_name)
        {
            Some(worktree) => Some(worktree.path),
            None => {
                eprintln!("Failed to find worktree {worktree_name}");
                None
            }
        },
        Err(error) => {
            eprintln!("Failed to list worktrees: {error}");
            None
        }
    }
}

fn delete_worktree_at_path(
    repo: &GitRepo,
    worktree_name: &str,
    worktree_path: &std::path::Path,
) -> anyhow::Result<bool> {
    let remove = run_git(
        repo.workdir(),
        [
            std::ffi::OsStr::new("worktree"),
            std::ffi::OsStr::new("remove"),
            std::ffi::OsStr::new("--force"),
            worktree_path.as_os_str(),
        ],
    );
    if remove.is_err() {
        // Path may have been deleted out-of-band; prune lingering admin data.
        let _ = run_git(repo.workdir(), ["worktree", "prune"]);
        let still_present = list_worktrees_raw(repo)
            .map(|worktrees| {
                worktrees
                    .iter()
                    .any(|worktree| worktree.name == worktree_name)
            })
            .unwrap_or(false);
        if still_present {
            eprintln!("Failed to delete worktree {worktree_name}");
            return Ok(false);
        }
    }
    Ok(true)
}

fn with_worktree_deletion_lock(
    lock_dir: &std::path::Path,
    worktree_path: &std::path::Path,
    delete: impl FnOnce() -> anyhow::Result<bool>,
) -> anyhow::Result<bool> {
    with_worktree_deletion_lock_with_wait(lock_dir, worktree_path, || {}, delete)
}

fn with_worktree_deletion_lock_with_wait(
    lock_dir: &std::path::Path,
    worktree_path: &std::path::Path,
    lock_waiting: impl FnOnce(),
    delete: impl FnOnce() -> anyhow::Result<bool>,
) -> anyhow::Result<bool> {
    let _deletion_lock = lock::try_lock_deletion(lock_dir, worktree_path)
        .map_err(|error| worktree_lock_error(error, worktree_path))?;
    let _lock = lock::lock_exclusive(lock_dir, worktree_path, lock_waiting)?;
    delete()
}

/// Delete a local branch if it exists. Returns true if deleted.
pub fn delete_branch_if_exists(repo: &GitRepo, branch: &str) -> bool {
    if branch.is_empty() {
        return false;
    }
    if !repo.local_branch_exists(branch) {
        return false;
    }
    repo.delete_branch(branch).is_ok()
}

/// Find the worktree name from its path.
pub fn find_worktree_name(repo: &GitRepo, worktree_path: &str) -> Result<String> {
    let normalized = worktree_path.trim_end_matches('/');
    let entries = list_worktrees_raw(repo).context("Failed to list worktrees")?;
    for entry in entries {
        let p = entry.path.to_string_lossy();
        if p.trim_end_matches('/') == normalized {
            return Ok(entry.name);
        }
    }
    Err(WmError::WorktreeNotFound(worktree_path.to_string()).into())
}

/// Basic information about a linked worktree.
#[derive(Debug, Clone)]
pub struct LinkedWorktree {
    pub name: String,
    pub path: PathBuf,
    pub branch: String,
    pub commit: String,
}

/// List all linked worktrees (excludes main worktree).
pub fn list_linked_worktrees(repo: &GitRepo) -> Result<Vec<LinkedWorktree>> {
    let all = list_worktrees_raw(repo).context("Failed to list worktrees")?;
    let mut result = Vec::new();
    // First entry from --porcelain is always the main worktree.
    for entry in all.into_iter().skip(1) {
        result.push(LinkedWorktree {
            name: entry.name,
            path: entry.path,
            branch: if entry.branch.is_empty() {
                "(unknown)".to_string()
            } else {
                entry.branch
            },
            commit: entry.short_hash,
        });
    }
    Ok(result)
}

/// Get the main worktree path.
pub fn get_main_worktree_path(repo: &GitRepo) -> Result<PathBuf> {
    if repo.is_worktree() {
        repo.main_workdir()
    } else {
        Ok(repo.workdir().to_path_buf())
    }
}

/// Raw worktree-list entry from `git worktree list --porcelain`.
#[derive(Debug, Clone)]
struct WorktreeEntry {
    /// Worktree name (last path component, matching git's internal name).
    name: String,
    path: PathBuf,
    /// Branch shorthand, `"(detached)"` for detached HEAD, or empty.
    branch: String,
    /// 7-char short hash.
    short_hash: String,
}

/// Run `git worktree list --porcelain -z` and parse the result. First entry is
/// the main worktree.
fn list_worktrees_raw(repo: &GitRepo) -> anyhow::Result<Vec<WorktreeEntry>> {
    let stdout = run_git(repo.workdir(), ["worktree", "list", "--porcelain", "-z"])?;
    Ok(parse_worktree_porcelain(&stdout))
}

fn parse_worktree_porcelain(text: &str) -> Vec<WorktreeEntry> {
    let mut out = Vec::new();
    let mut current: Option<(PathBuf, String, String, bool)> = None;

    fn flush(cur: Option<(PathBuf, String, String, bool)>, out: &mut Vec<WorktreeEntry>) {
        if let Some((path, head, branch, detached)) = cur {
            let name = path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let short_hash = head.chars().take(7).collect::<String>();
            let branch_display = if detached {
                "(detached)".to_string()
            } else {
                branch
                    .strip_prefix("refs/heads/")
                    .unwrap_or(&branch)
                    .to_string()
            };
            out.push(WorktreeEntry {
                name,
                path,
                branch: branch_display,
                short_hash,
            });
        }
    }

    for line in text.split('\0') {
        if line.is_empty() {
            flush(current.take(), &mut out);
            continue;
        }
        if let Some(rest) = line.strip_prefix("worktree ") {
            flush(current.take(), &mut out);
            current = Some((PathBuf::from(rest), String::new(), String::new(), false));
        } else if let Some(rest) = line.strip_prefix("HEAD ") {
            if let Some(c) = current.as_mut() {
                c.1 = rest.to_string();
            }
        } else if let Some(rest) = line.strip_prefix("branch ") {
            if let Some(c) = current.as_mut() {
                c.2 = rest.to_string();
            }
        } else if line == "detached"
            && let Some(c) = current.as_mut()
        {
            c.3 = true;
        }
    }
    flush(current.take(), &mut out);
    out
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::shared::testing::TestRepo;
    use rstest::rstest;

    #[rstest]
    #[case::clean_without_force("", false, true, (true, false, true, true, true))]
    #[case::tracked_change_without_force(" M tracked.txt\0", false, true, (false, true, true, false, false))]
    #[case::untracked_file_without_force("?? scratch.txt\0", false, true, (false, true, true, false, false))]
    #[case::untracked_file_with_force("?? scratch.txt\0", true, true, (true, false, false, true, true))]
    #[case::missing_worktree_skips_status("?? scratch.txt\0", false, false, (true, false, false, true, true))]
    fn delete_worktree_checks_status_before_removal(
        #[case] status: &str,
        #[case] force: bool,
        #[case] worktree_exists: bool,
        #[case] expected: (bool, bool, bool, bool, bool),
    ) {
        let directory = tempfile::tempdir().expect("temporary worktree should be created");
        let lock_dir = tempfile::tempdir().expect("temporary lock directory should be created");
        let status_queried = Cell::new(false);
        let before_delete_called = Cell::new(false);
        let removed = Cell::new(false);
        let missing_path = directory.path().join("missing");

        let worktree_path = if worktree_exists {
            directory.path()
        } else {
            &missing_path
        };
        let result = delete_worktree_if_clean_with(
            worktree_path,
            force,
            lock_dir.path(),
            |_| {
                status_queried.set(true);
                Ok(status.to_string())
            },
            || {
                before_delete_called.set(true);
                Ok(())
            },
            || {
                removed.set(true);
                Ok(true)
            },
            || {},
        );
        let rejected_as_dirty = result
            .as_ref()
            .err()
            .is_some_and(|error| error.downcast_ref::<DirtyWorktreeError>().is_some());

        assert_eq!(
            (
                result.is_ok(),
                rejected_as_dirty,
                status_queried.get(),
                before_delete_called.get(),
                removed.get()
            ),
            expected,
        );
    }

    #[test]
    fn uncommitted_files_use_the_status_paths_including_renames() {
        let directory = tempfile::tempdir().expect("temporary worktree should be created");
        let lock_dir = tempfile::tempdir().expect("temporary lock directory should be created");
        let files = uncommitted_files_with(directory.path(), lock_dir.path(), |_| {
            Ok(" M src/changed.rs\0R  src/renamed.rs\0src/old.rs\0?? src/new.rs\0".into())
        })
        .expect("status should be parsed");

        assert_eq!(
            files,
            vec!["src/changed.rs", "src/renamed.rs", "src/new.rs",],
        );
    }

    #[test]
    fn dirty_checks_report_deletion_in_progress_without_running_status() {
        let worktree = tempfile::tempdir().expect("temporary worktree should be created");
        let lock_dir = tempfile::tempdir().expect("temporary lock directory should be created");
        let _lock = lock::lock_exclusive(lock_dir.path(), worktree.path(), || {})
            .expect("exclusive worktree lock should be acquired");
        let status_calls = Cell::new(0);

        let ensure_result =
            ensure_clean_for_deletion_with(worktree.path(), false, lock_dir.path(), |_| {
                status_calls.set(status_calls.get() + 1);
                Ok(String::new())
            });
        let files_result = uncommitted_files_with(worktree.path(), lock_dir.path(), |_| {
            status_calls.set(status_calls.get() + 1);
            Ok(String::new())
        });
        let observed = (
            ensure_result
                .as_ref()
                .err()
                .and_then(|error| error.downcast_ref::<WorktreeDeletionInProgressError>())
                .is_some(),
            files_result
                .as_ref()
                .err()
                .and_then(|error| error.downcast_ref::<WorktreeDeletionInProgressError>())
                .is_some(),
            status_calls.get(),
        );

        assert_eq!(observed, (true, true, 0));
    }

    #[test]
    fn delete_rechecks_worktree_after_waiting_for_status_lock() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let worktree = tempfile::tempdir().expect("temporary worktree should be created");
        let lock_dir = tempfile::tempdir().expect("temporary lock directory should be created");
        let shared = lock::try_lock_shared(lock_dir.path(), worktree.path())
            .expect("shared worktree lock should be acquired");
        let status_became_dirty = std::sync::Arc::new(AtomicBool::new(false));
        let deleted = std::sync::Arc::new(AtomicBool::new(false));
        let thread_status = std::sync::Arc::clone(&status_became_dirty);
        let thread_deleted = std::sync::Arc::clone(&deleted);
        let thread_worktree = worktree.path().to_path_buf();
        let thread_lock_dir = lock_dir.path().to_path_buf();
        let (started_sender, started_receiver) = std::sync::mpsc::channel();
        let (waiting_sender, waiting_receiver) = std::sync::mpsc::channel();
        let (result_sender, result_receiver) = std::sync::mpsc::channel();
        let delete_thread = std::thread::spawn(move || {
            started_sender.send(()).expect("started signal should send");
            let result = delete_worktree_if_clean_with(
                &thread_worktree,
                false,
                &thread_lock_dir,
                |_| {
                    Ok(if thread_status.load(Ordering::SeqCst) {
                        " M saved-during-wait.txt\0".to_string()
                    } else {
                        String::new()
                    })
                },
                || Ok(()),
                || {
                    thread_deleted.store(true, Ordering::SeqCst);
                    Ok(true)
                },
                || {
                    waiting_sender
                        .send(())
                        .expect("lock wait signal should send");
                },
            );
            result_sender
                .send(result)
                .expect("deletion result should send");
        });
        started_receiver.recv().expect("delete thread should start");

        let lock_is_waiting = waiting_receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .is_ok();
        status_became_dirty.store(true, Ordering::SeqCst);
        drop(shared);

        let result = result_receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("deletion should finish after the status lock is released");
        delete_thread.join().expect("delete thread should finish");
        let rejected_as_dirty = result
            .as_ref()
            .err()
            .is_some_and(|error| error.downcast_ref::<DirtyWorktreeError>().is_some());

        assert_eq!(
            (
                lock_is_waiting,
                rejected_as_dirty,
                deleted.load(Ordering::SeqCst)
            ),
            (true, true, false),
        );
    }

    #[test]
    fn shared_worktree_locks_allow_concurrent_dirty_checks() {
        let worktree = tempfile::tempdir().expect("temporary worktree should be created");
        let lock_dir = tempfile::tempdir().expect("temporary lock directory should be created");

        let first = lock::try_lock_shared(lock_dir.path(), worktree.path())
            .expect("first shared worktree lock should be acquired");
        let second = lock::try_lock_shared(lock_dir.path(), worktree.path())
            .expect("second shared worktree lock should be acquired");

        assert_eq!(
            (first.metadata().is_ok(), second.metadata().is_ok()),
            (true, true)
        );
    }

    #[test]
    fn worktree_path_aliases_share_the_same_lock() {
        let directory = tempfile::tempdir().expect("temporary parent should be created");
        let worktree = directory.path().join("worktree");
        let alias_parent = directory.path().join("nested");
        std::fs::create_dir_all(&worktree).expect("temporary worktree should be created");
        std::fs::create_dir(&alias_parent).expect("alias parent should be created");
        let alias = alias_parent.join("..").join("worktree");
        let lock_dir = tempfile::tempdir().expect("temporary lock directory should be created");
        let _lock = lock::lock_exclusive(lock_dir.path(), &worktree, || {})
            .expect("exclusive worktree lock should be acquired");

        let alias_lock = lock::try_lock_shared(lock_dir.path(), &alias);

        let observed = match alias_lock {
            Err(lock::WorktreeLockError::Busy) => "busy",
            Err(lock::WorktreeLockError::Io(_)) => "io error",
            Ok(_) => "available",
        };

        assert_eq!(observed, "busy");
    }

    #[test]
    fn deletion_lock_waits_for_an_active_dirty_check() {
        let worktree = tempfile::tempdir().expect("temporary worktree should be created");
        let lock_dir = tempfile::tempdir().expect("temporary lock directory should be created");
        let shared = lock::try_lock_shared(lock_dir.path(), worktree.path())
            .expect("shared worktree lock should be acquired");
        let thread_lock_dir = lock_dir.path().to_path_buf();
        let thread_worktree = worktree.path().to_path_buf();
        let (started_sender, started_receiver) = std::sync::mpsc::channel();
        let (locked_sender, locked_receiver) = std::sync::mpsc::channel();
        let delete_thread = std::thread::spawn(move || {
            started_sender.send(()).expect("started signal should send");
            with_worktree_deletion_lock(&thread_lock_dir, &thread_worktree, || {
                locked_sender.send(()).expect("locked signal should send");
                Ok(true)
            })
            .expect("worktree deletion should complete");
        });
        started_receiver.recv().expect("delete thread should start");

        let blocked_before_release = matches!(
            locked_receiver.recv_timeout(std::time::Duration::from_millis(50)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        );
        drop(shared);
        let acquired_after_release = locked_receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .is_ok();
        delete_thread.join().expect("delete thread should finish");

        assert_eq!(
            (blocked_before_release, acquired_after_release),
            (true, true)
        );
    }

    #[test]
    fn competing_deletions_return_an_in_progress_error_without_waiting() {
        let worktree = tempfile::tempdir().expect("temporary worktree should be created");
        let lock_dir = tempfile::tempdir().expect("temporary lock directory should be created");
        let first_deletion = lock::try_lock_deletion(lock_dir.path(), worktree.path())
            .expect("first deletion marker should be acquired");
        let thread_lock_dir = lock_dir.path().to_path_buf();
        let thread_worktree = worktree.path().to_path_buf();
        let (result_sender, result_receiver) = std::sync::mpsc::channel();
        let delete_thread = std::thread::spawn(move || {
            let result =
                with_worktree_deletion_lock(&thread_lock_dir, &thread_worktree, || Ok(true));
            let outcome = match result {
                Err(error)
                    if error
                        .downcast_ref::<WorktreeDeletionInProgressError>()
                        .is_some() =>
                {
                    "busy"
                }
                Err(_) => "other error",
                Ok(_) => "deleted",
            };
            result_sender.send(outcome).expect("result should be sent");
        });
        let returned_without_waiting = result_receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .ok();
        let did_return_without_waiting = returned_without_waiting.is_some();
        drop(first_deletion);
        let outcome = returned_without_waiting.or_else(|| {
            result_receiver
                .recv_timeout(std::time::Duration::from_secs(2))
                .ok()
        });
        delete_thread.join().expect("delete thread should finish");

        assert_eq!((did_return_without_waiting, outcome), (true, Some("busy")));
    }

    #[rstest]
    #[case::simple("feature-branch", BRANCH_PREFIX, "feature-branch")]
    #[case::with_prefix("fohte/feature-branch", BRANCH_PREFIX, "feature-branch")]
    #[case::with_slash("feature/branch", BRANCH_PREFIX, "feature-branch")]
    #[case::with_prefix_and_slash("fohte/feature/branch", BRANCH_PREFIX, "feature-branch")]
    #[case::nested_slash("feature/sub/branch", BRANCH_PREFIX, "feature-sub-branch")]
    #[case::custom_prefix("user/feature-branch", "user/", "feature-branch")]
    #[case::custom_prefix_with_slash("user/feature/sub", "user/", "feature-sub")]
    #[case::no_match("feature-branch", "other/", "feature-branch")]
    #[case::empty_prefix("fohte/feature", "", "fohte-feature")]
    fn test_branch_to_worktree_name(
        #[case] branch: &str,
        #[case] prefix: &str,
        #[case] expected: &str,
    ) {
        assert_eq!(branch_to_worktree_name(branch, prefix), expected);
    }

    #[test]
    fn get_main_repo_from_main_returns_same_repo() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        let main_repo = get_main_repo(&repo).unwrap();
        assert_eq!(
            main_repo.workdir().canonicalize().unwrap(),
            test_repo.path()
        );
    }

    #[test]
    fn get_main_repo_from_worktree_returns_main() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("feature");

        let wt_repo = crate::infra::git::open_repo_at(&test_repo.worktree_path("feature")).unwrap();
        let main_repo = get_main_repo(&wt_repo).unwrap();

        assert_eq!(
            main_repo.workdir().canonicalize().unwrap(),
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

        assert!(
            list_worktrees_raw(&repo)
                .unwrap()
                .iter()
                .any(|w| w.name == "to-delete")
        );

        let result = delete_worktree(&repo, "to-delete").unwrap();
        assert!(result);

        assert!(
            !list_worktrees_raw(&repo)
                .unwrap()
                .iter()
                .any(|w| w.name == "to-delete")
        );
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

        let mut names: Vec<&str> = worktrees.iter().map(|w| w.name.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["feature-a", "feature-b"]);
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

        let wt_repo = crate::infra::git::open_repo_at(&test_repo.worktree_path("feature")).unwrap();
        let path = get_main_worktree_path(&wt_repo).unwrap();

        assert_eq!(path.canonicalize().unwrap(), test_repo.path());
    }

    #[test]
    fn delete_branch_if_exists_works_after_worktree_deleted() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("feature-to-delete");

        let repo = test_repo.open();
        let wt_path = test_repo.worktree_path("feature-to-delete");

        assert!(
            repo.local_branch_exists("feature-to-delete"),
            "Branch should exist before deletion"
        );

        struct DirGuard(std::path::PathBuf);
        impl Drop for DirGuard {
            fn drop(&mut self) {
                let _ = std::env::set_current_dir(&self.0);
            }
        }

        let branch_deleted = {
            let _guard = DirGuard(std::env::current_dir().unwrap());
            std::env::set_current_dir(&wt_path).unwrap();

            let deleted = delete_worktree(&repo, "feature-to-delete").unwrap();
            assert!(deleted, "Worktree should be deleted");

            delete_branch_if_exists(&repo, "feature-to-delete")
        };

        assert!(
            branch_deleted,
            "Branch should be deleted even after worktree is removed"
        );

        assert!(
            !repo.local_branch_exists("feature-to-delete"),
            "Branch should not exist after deletion"
        );
    }

    #[test]
    fn parse_worktree_porcelain_handles_basic_and_unusual_paths() {
        let unusual_path = format!("/repo/.worktrees/feature branch{}preview", char::from(10));
        let input = format!(
            "worktree /repo\0HEAD abcdef0123456789\0branch refs/heads/main\0\0\
             worktree {unusual_path}\0HEAD fedcba9876543210\0branch refs/heads/feature\0\0\
             worktree /repo/.worktrees/detached\0HEAD 1111111222222222\0detached\0\0"
        );
        let entries = parse_worktree_porcelain(&input);
        assert_eq!(
            entries
                .into_iter()
                .map(|entry| (entry.name, entry.path, entry.branch, entry.short_hash))
                .collect::<Vec<_>>(),
            vec![
                (
                    "repo".to_string(),
                    PathBuf::from("/repo"),
                    "main".to_string(),
                    "abcdef0".to_string(),
                ),
                (
                    format!("feature branch{}preview", char::from(10)),
                    PathBuf::from(unusual_path),
                    "feature".to_string(),
                    "fedcba9".to_string(),
                ),
                (
                    "detached".to_string(),
                    PathBuf::from("/repo/.worktrees/detached"),
                    "(detached)".to_string(),
                    "1111111".to_string(),
                ),
            ],
        );
    }
}
