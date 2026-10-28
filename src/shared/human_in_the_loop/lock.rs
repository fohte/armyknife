use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use crate::infra::process::is_process_alive;

use super::error::{HumanInTheLoopError, Result};
use crate::infra::tmux;

const LOCK_FILE_MAGIC: &str = "armyknife-review-lock-v1";

/// RAII guard for the lock file owned by a review launcher.
pub struct LockGuard {
    lock_path: PathBuf,
    lock_id: String,
    holder_pid: u32,
    disarmed: bool,
}

impl LockGuard {
    /// Create a lock file and return a guard.
    pub fn acquire(document_path: &Path) -> Result<Self> {
        let lock_path = Self::lock_path(document_path);
        let holder_pid = std::process::id();
        let lock_id = uuid::Uuid::new_v4().to_string();
        let record = LockRecord {
            id: lock_id.clone(),
            pids: vec![holder_pid],
        };

        loop {
            let staging_path = write_staging_record(&lock_path, &record)?;
            match fs::hard_link(&staging_path, &lock_path) {
                Ok(()) => {
                    let _ = fs::remove_file(staging_path);
                    sync_parent_dir(&lock_path);
                    return Ok(Self {
                        lock_path,
                        lock_id,
                        holder_pid,
                        disarmed: false,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    let _ = fs::remove_file(staging_path);
                    match try_remove_stale_lock(&lock_path) {
                        StaleRemoval::Removed | StaleRemoval::Replaced => continue,
                        StaleRemoval::Active => return Err(error.into()),
                    }
                }
                Err(error) => {
                    let _ = fs::remove_file(staging_path);
                    return Err(error.into());
                }
            }
        }
    }

    /// Check whether a live review holder exists for the given document.
    pub fn is_locked(document_path: &Path) -> bool {
        matches!(
            inspect_lock(&Self::lock_path(document_path), is_process_alive),
            LockState::Active
        )
    }

    /// Get the lock file path for a document.
    pub fn lock_path(document_path: &Path) -> PathBuf {
        let mut lock_path = document_path.as_os_str().to_os_string();
        lock_path.push(".lock");
        PathBuf::from(lock_path)
    }

    /// Return the identifier that the review-complete child must present.
    pub fn lock_id(&self) -> &str {
        &self.lock_id
    }

    /// Prevent this guard from removing the lock file on drop.
    pub fn disarm(&mut self) {
        self.disarmed = true;
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if !self.disarmed {
            release_holder(&self.lock_path, &self.lock_id, self.holder_pid);
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct LockRecord {
    id: String,
    pids: Vec<u32>,
}

impl LockRecord {
    fn serialize(&self) -> Vec<u8> {
        let mut contents = String::new();
        contents.push_str(LOCK_FILE_MAGIC);
        contents.push('\n');
        contents.push_str("id=");
        contents.push_str(&self.id);
        contents.push('\n');
        for pid in &self.pids {
            contents.push_str("pid=");
            contents.push_str(&pid.to_string());
            contents.push('\n');
        }
        contents.into_bytes()
    }

    fn parse(contents: &[u8]) -> Option<Self> {
        if !contents.ends_with(b"\n") {
            return None;
        }

        let contents = std::str::from_utf8(contents).ok()?;
        let mut lines = contents.lines();
        if lines.next()? != LOCK_FILE_MAGIC {
            return None;
        }

        let id = lines.next()?.strip_prefix("id=")?.to_string();
        if id.is_empty() {
            return None;
        }

        let mut pids = Vec::new();
        for line in lines {
            let pid = line.strip_prefix("pid=")?.parse::<u32>().ok()?;
            if pid == 0 {
                return None;
            }
            pids.push(pid);
        }

        if pids.is_empty() {
            return None;
        }

        Some(Self { id, pids })
    }
}

fn write_staging_record(lock_path: &Path, record: &LockRecord) -> io::Result<PathBuf> {
    let mut staging_path = lock_path.as_os_str().to_os_string();
    staging_path.push(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let staging_path = PathBuf::from(staging_path);
    let mut staging_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&staging_path)?;

    if let Err(error) = staging_file
        .write_all(&record.serialize())
        .and_then(|()| staging_file.sync_all())
    {
        let _ = fs::remove_file(&staging_path);
        return Err(error);
    }

    Ok(staging_path)
}

fn inspect_lock(path: &Path, mut is_alive: impl FnMut(u32) -> bool) -> LockState {
    for _ in 0..8 {
        let file = match open_existing_lock(path) {
            Ok(Some(file)) => file,
            Ok(None) => return LockState::Missing,
            Err(_) => return LockState::Active,
        };
        let Ok(_file_lock) = AdvisoryLock::exclusive(&file) else {
            return LockState::Active;
        };
        if !same_file_at_path(&file, path) {
            continue;
        }

        let record = match read_record(&file) {
            Ok(Some(record)) => record,
            Ok(None) | Err(_) => return LockState::Active,
        };
        if !same_file_at_path(&file, path) {
            continue;
        }

        return if record.pids.iter().any(|pid| is_alive(*pid)) {
            LockState::Active
        } else {
            LockState::Stale
        };
    }

    LockState::Active
}

fn open_existing_lock(path: &Path) -> io::Result<Option<File>> {
    match OpenOptions::new().read(true).write(true).open(path) {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => match fs::symlink_metadata(path) {
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "review lock path is not a readable file",
            )),
            Err(path_error) if path_error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(path_error) => Err(path_error),
        },
        Err(error) => Err(error),
    }
}

fn read_record(file: &File) -> io::Result<Option<LockRecord>> {
    let mut reader = file.try_clone()?;
    let mut contents = Vec::new();
    reader.read_to_end(&mut contents)?;
    Ok(LockRecord::parse(&contents))
}

enum LockState {
    Missing,
    Active,
    Stale,
}

enum StaleRemoval {
    Removed,
    Replaced,
    Active,
}

fn try_remove_stale_lock(path: &Path) -> StaleRemoval {
    let file = match open_existing_lock(path) {
        Ok(Some(file)) => file,
        Ok(None) => return StaleRemoval::Replaced,
        Err(_) => return StaleRemoval::Active,
    };
    let Ok(_file_lock) = AdvisoryLock::exclusive(&file) else {
        return StaleRemoval::Active;
    };
    if !same_file_at_path(&file, path) {
        return StaleRemoval::Active;
    }

    let record = match read_record(&file) {
        Ok(Some(record)) => record,
        Ok(None) | Err(_) => return StaleRemoval::Active,
    };
    if record.pids.iter().any(|pid| is_process_alive(*pid)) {
        return StaleRemoval::Active;
    }
    if !same_file_at_path(&file, path) {
        return StaleRemoval::Active;
    }

    match fs::remove_file(path) {
        Ok(()) => {
            sync_parent_dir(path);
            StaleRemoval::Removed
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => StaleRemoval::Replaced,
        Err(_) => StaleRemoval::Active,
    }
}

fn register_holder(path: &Path, lock_id: &str, holder_pid: u32) -> Result<()> {
    let file = open_existing_lock(path)?.ok_or(HumanInTheLoopError::ReviewLockLost)?;
    let _file_lock = AdvisoryLock::exclusive(&file)?;
    if !same_file_at_path(&file, path) {
        return Err(HumanInTheLoopError::ReviewLockLost);
    }

    let mut record = read_record(&file)?.ok_or(HumanInTheLoopError::ReviewLockLost)?;
    if record.id != lock_id {
        return Err(HumanInTheLoopError::ReviewLockLost);
    }
    if record.pids.contains(&holder_pid) {
        return Ok(());
    }

    record.pids.push(holder_pid);
    replace_record(path, &file, &record)?;
    Ok(())
}

fn release_holder(path: &Path, lock_id: &str, holder_pid: u32) {
    release_holder_with(path, lock_id, holder_pid, is_process_alive);
}

fn release_holder_with(
    path: &Path,
    lock_id: &str,
    holder_pid: u32,
    mut is_alive: impl FnMut(u32) -> bool,
) {
    let Ok(Some(file)) = open_existing_lock(path) else {
        return;
    };
    let Ok(_file_lock) = AdvisoryLock::exclusive(&file) else {
        return;
    };
    if !same_file_at_path(&file, path) {
        return;
    }

    let Ok(Some(record)) = read_record(&file) else {
        return;
    };
    if record.id != lock_id || !record.pids.contains(&holder_pid) {
        return;
    }

    let remaining_pids: Vec<_> = record
        .pids
        .into_iter()
        .filter(|pid| *pid != holder_pid && is_alive(*pid))
        .collect();
    if remaining_pids.is_empty() {
        if same_file_at_path(&file, path) && fs::remove_file(path).is_ok() {
            sync_parent_dir(path);
        }
        return;
    }

    let remaining = LockRecord {
        id: record.id,
        pids: remaining_pids,
    };
    let _ = replace_record(path, &file, &remaining);
}

fn replace_record(path: &Path, current_file: &File, record: &LockRecord) -> io::Result<()> {
    let staging_path = write_staging_record(path, record)?;
    let original_permissions = match current_file.metadata() {
        Ok(metadata) => metadata.permissions(),
        Err(error) => {
            let _ = fs::remove_file(&staging_path);
            return Err(error);
        }
    };
    if let Err(error) = fs::set_permissions(&staging_path, original_permissions) {
        let _ = fs::remove_file(&staging_path);
        return Err(error);
    }
    if !same_file_at_path(current_file, path) {
        let _ = fs::remove_file(&staging_path);
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "review lock was replaced",
        ));
    }

    if let Err(error) = fs::rename(&staging_path, path) {
        let _ = fs::remove_file(&staging_path);
        return Err(error);
    }
    sync_parent_dir(path);
    Ok(())
}

fn same_file_at_path(file: &File, path: &Path) -> bool {
    match (file.metadata(), fs::symlink_metadata(path)) {
        (Ok(owned), Ok(current)) => same_file(&owned, &current),
        _ => false,
    }
}

fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.dev() == right.dev() && left.ino() == right.ino()
}

fn sync_parent_dir(path: &Path) {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    if let Ok(directory) = File::open(parent) {
        let _ = directory.sync_all();
    }
}

struct AdvisoryLock<'a> {
    file: &'a File,
}

impl<'a> AdvisoryLock<'a> {
    fn exclusive(file: &'a File) -> io::Result<Self> {
        // SAFETY: file remains open for the lifetime of this guard.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
            Ok(Self { file })
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

impl Drop for AdvisoryLock<'_> {
    fn drop(&mut self) {
        // SAFETY: file remains open for the lifetime of this guard.
        let _ = unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
    }
}

/// Child-side cleanup guard for normal review completion.
///
/// The child registers itself before opening the editor. The lock remains
/// active until both the launcher and editor process have exited.
pub struct CleanupGuard {
    lock_path: PathBuf,
    lock_id: String,
    holder_pid: u32,
    tmux_target: Option<String>,
}

impl CleanupGuard {
    pub fn new(document_path: &Path, lock_id: &str, tmux_target: Option<String>) -> Result<Self> {
        let lock_path = LockGuard::lock_path(document_path);
        let holder_pid = std::process::id();
        register_holder(&lock_path, lock_id, holder_pid)?;
        Ok(Self {
            lock_path,
            lock_id: lock_id.to_string(),
            holder_pid,
            tmux_target,
        })
    }
}

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        release_holder(&self.lock_path, &self.lock_id, self.holder_pid);

        if let Some(ref target) = self.tmux_target {
            let _ = tmux::focus_pane(target);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    struct ReviewDocumentFixture {
        _temp_dir: TempDir,
        path: PathBuf,
    }

    #[fixture]
    fn review_document() -> ReviewDocumentFixture {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.md");
        fs::write(&path, "content").unwrap();
        ReviewDocumentFixture {
            _temp_dir: temp_dir,
            path,
        }
    }

    fn write_record(path: &Path, id: &str, pids: &[u32]) {
        let record = LockRecord {
            id: id.to_string(),
            pids: pids.to_vec(),
        };
        fs::write(path, record.serialize()).unwrap();
    }

    #[rstest]
    #[case::markdown("file.md", "file.md.lock")]
    #[case::text("file.txt", "file.txt.lock")]
    #[case::extensionless("file", "file.lock")]
    #[case::compound_extension("file.tar.gz", "file.tar.gz.lock")]
    fn lock_path_appends_lock_extension(#[case] filename: &str, #[case] expected: &str) {
        let path = PathBuf::from(filename);
        let lock_path = LockGuard::lock_path(&path);
        assert_eq!(lock_path, PathBuf::from(expected));
    }

    #[rstest]
    fn lock_guard_creates_and_removes_lock_file(review_document: ReviewDocumentFixture) {
        let doc_path = &review_document.path;
        let lock_path = LockGuard::lock_path(doc_path);

        let initially_locked = LockGuard::is_locked(doc_path);
        let while_held;
        {
            let _guard = LockGuard::acquire(doc_path).unwrap();
            while_held = (lock_path.exists(), LockGuard::is_locked(doc_path));
        }
        let after_drop = (lock_path.exists(), LockGuard::is_locked(doc_path));

        assert_eq!(
            (initially_locked, while_held, after_drop),
            (false, (true, true), (false, false)),
        );
    }

    #[rstest]
    fn lock_guard_does_not_replace_a_live_lock(review_document: ReviewDocumentFixture) {
        let parent_guard = LockGuard::acquire(&review_document.path).unwrap();
        let competing_acquire = LockGuard::acquire(&review_document.path)
            .map(|_| ())
            .map_err(|error| {
                matches!(
                    error,
                    HumanInTheLoopError::Io(ref io_error)
                        if io_error.kind() == io::ErrorKind::AlreadyExists
                )
            });

        assert_eq!(
            (
                LockGuard::is_locked(&review_document.path),
                competing_acquire,
            ),
            (true, Err(true)),
        );
        drop(parent_guard);
    }

    #[rstest]
    fn lock_guard_does_not_replace_an_existing_lock(review_document: ReviewDocumentFixture) {
        let doc_path = &review_document.path;
        let lock_path = LockGuard::lock_path(doc_path);
        fs::write(&lock_path, "legacy lock").unwrap();
        let acquired = LockGuard::acquire(doc_path).is_ok();
        let contents = fs::read_to_string(lock_path).unwrap();

        assert_eq!((acquired, contents), (false, "legacy lock".into()));
    }

    #[rstest]
    fn stale_lock_is_unlocked_and_replaced(review_document: ReviewDocumentFixture) {
        let doc_path = &review_document.path;
        let lock_path = LockGuard::lock_path(doc_path);
        write_record(&lock_path, "expired-lock", &[u32::MAX]);
        let was_locked = LockGuard::is_locked(doc_path);

        let guard = LockGuard::acquire(doc_path).unwrap();
        let record = LockRecord::parse(&fs::read(&lock_path).unwrap()).unwrap();
        let replaced = record.id == guard.lock_id() && record.pids == [std::process::id()];

        assert_eq!((was_locked, replaced), (false, true));
    }

    #[rstest]
    #[case::empty("")]
    #[case::legacy("legacy lock")]
    #[case::partial(indoc! {"
        armyknife-review-lock-v1
        id=unfinished
        pid=
    "})]
    fn unknown_lock_contents_remain_active(
        review_document: ReviewDocumentFixture,
        #[case] contents: &str,
    ) {
        let lock_path = LockGuard::lock_path(&review_document.path);
        fs::write(&lock_path, contents).unwrap();

        assert!(LockGuard::is_locked(&review_document.path));
    }

    #[rstest]
    fn parent_release_keeps_child_lock_until_child_exits(review_document: ReviewDocumentFixture) {
        let lock_path = LockGuard::lock_path(&review_document.path);
        let parent_guard = LockGuard::acquire(&review_document.path).unwrap();
        let parent_pid = parent_guard.holder_pid;
        register_holder(&lock_path, parent_guard.lock_id(), 202).unwrap();
        let after_register = LockRecord::parse(&fs::read(&lock_path).unwrap()).unwrap();

        release_holder_with(&lock_path, parent_guard.lock_id(), parent_pid, |pid| {
            pid == 202
        });
        let after_parent = LockRecord::parse(&fs::read(&lock_path).unwrap()).unwrap();
        let locked_after_parent = inspect_lock(&lock_path, |pid| pid == 202);
        release_holder_with(&lock_path, parent_guard.lock_id(), 202, |_| false);

        assert_eq!(
            (
                after_register.pids,
                after_parent.pids,
                matches!(locked_after_parent, LockState::Active),
                lock_path.exists(),
            ),
            (vec![parent_pid, 202], vec![202], true, false),
        );
    }

    #[rstest]
    fn lock_guard_does_not_remove_a_replacement_lock(review_document: ReviewDocumentFixture) {
        let doc_path = &review_document.path;
        let old_guard = LockGuard::acquire(doc_path).unwrap();
        fs::remove_file(LockGuard::lock_path(doc_path)).unwrap();
        let new_guard = LockGuard::acquire(doc_path).unwrap();
        drop(old_guard);
        let replacement_remains = LockGuard::is_locked(doc_path);
        drop(new_guard);

        assert_eq!(
            (replacement_remains, LockGuard::is_locked(doc_path)),
            (true, false),
        );
    }

    #[rstest]
    fn lock_guard_disarm_prevents_cleanup(review_document: ReviewDocumentFixture) {
        let doc_path = &review_document.path;
        let lock_path = LockGuard::lock_path(doc_path);

        let while_held;
        {
            let mut guard = LockGuard::acquire(doc_path).unwrap();
            while_held = lock_path.exists();
            guard.disarm();
        }

        assert_eq!((while_held, lock_path.exists()), (true, true));
    }

    #[rstest]
    fn cleanup_guard_removes_lock_file(review_document: ReviewDocumentFixture) {
        let doc_path = &review_document.path;
        let lock_path = LockGuard::lock_path(doc_path);
        let parent_guard = LockGuard::acquire(doc_path).unwrap();

        let existed_before_cleanup = lock_path.exists();
        {
            let _guard = CleanupGuard::new(doc_path, parent_guard.lock_id(), None).unwrap();
        }
        drop(parent_guard);

        assert_eq!((existed_before_cleanup, lock_path.exists()), (true, false));
    }

    #[rstest]
    fn cleanup_guard_rejects_a_replaced_review_lock(review_document: ReviewDocumentFixture) {
        let old_guard = LockGuard::acquire(&review_document.path).unwrap();
        let old_lock_id = old_guard.lock_id().to_string();
        fs::remove_file(LockGuard::lock_path(&review_document.path)).unwrap();
        let new_guard = LockGuard::acquire(&review_document.path).unwrap();

        let result = CleanupGuard::new(&review_document.path, &old_lock_id, None)
            .map(|_| ())
            .map_err(|error| error.to_string());

        assert_eq!(
            (result, LockGuard::is_locked(&review_document.path)),
            (
                Err("Review lock is no longer active; refusing to open the editor".into()),
                true,
            ),
        );
        drop(old_guard);
        drop(new_guard);
    }

    #[rstest]
    fn symlink_lock_is_conservatively_treated_as_active(review_document: ReviewDocumentFixture) {
        use std::os::unix::fs::symlink;

        let lock_path = LockGuard::lock_path(&review_document.path);
        let target_path = review_document.path.with_extension("lock-target");
        write_record(&target_path, "target-lock", &[u32::MAX]);
        symlink(&target_path, &lock_path).unwrap();

        assert_eq!(
            (
                LockGuard::is_locked(&review_document.path),
                LockGuard::acquire(&review_document.path).is_err(),
            ),
            (true, true),
        );
    }
}
