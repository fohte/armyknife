use std::fs;
use std::path::{Path, PathBuf};

use super::error::Result;
use crate::infra::tmux;

/// RAII guard for the lock file owned by a review launcher.
pub struct LockGuard {
    lock_path: PathBuf,
    lock_file: fs::File,
    disarmed: bool,
}

impl LockGuard {
    /// Create a lock file and return a guard.
    pub fn acquire(document_path: &Path) -> Result<Self> {
        let lock_path = Self::lock_path(document_path);
        let lock_file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)?;
        Ok(Self {
            lock_path,
            lock_file,
            disarmed: false,
        })
    }

    /// Check if a lock file exists for the given document.
    pub fn is_locked(document_path: &Path) -> bool {
        Self::lock_path(document_path).exists()
    }

    /// Get the lock file path for a document.
    pub fn lock_path(document_path: &Path) -> PathBuf {
        let mut lock_path = document_path.as_os_str().to_os_string();
        lock_path.push(".lock");
        PathBuf::from(lock_path)
    }

    /// Prevent this guard from removing the lock file on drop.
    pub fn disarm(&mut self) {
        self.disarmed = true;
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if !self.disarmed
            && let (Ok(owned), Ok(path)) = (
                self.lock_file.metadata(),
                fs::symlink_metadata(&self.lock_path),
            )
            && same_file(&owned, &path)
        {
            let _ = fs::remove_file(&self.lock_path);
        }
    }
}

fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    left.dev() == right.dev() && left.ino() == right.ino()
}

/// RAII guard for cleanup after review-complete (lock file + tmux restore).
///
/// This guard is used in the review-complete process to ensure:
/// 1. The lock file is always removed
/// 2. The tmux session is restored (if applicable)
pub struct CleanupGuard {
    lock_path: PathBuf,
    tmux_target: Option<String>,
}

impl CleanupGuard {
    pub fn new(document_path: &Path, tmux_target: Option<String>) -> Self {
        Self {
            lock_path: LockGuard::lock_path(document_path),
            tmux_target,
        }
    }
}

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        // Remove lock file
        let _ = fs::remove_file(&self.lock_path);

        restore_tmux_focus(self.tmux_target.as_deref());
    }
}

fn restore_tmux_focus(tmux_target: Option<&str>) {
    if let Some(target) = tmux_target {
        let _ = tmux::focus_pane(target);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use tempfile::TempDir;

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

    #[test]
    fn lock_guard_creates_and_removes_lock_file() {
        let temp_dir = TempDir::new().unwrap();
        let doc_path = temp_dir.path().join("test.md");
        fs::write(&doc_path, "content").unwrap();

        let lock_path = LockGuard::lock_path(&doc_path);

        let initially_locked = LockGuard::is_locked(&doc_path);
        let while_held;
        {
            let _guard = LockGuard::acquire(&doc_path).unwrap();
            while_held = (lock_path.exists(), LockGuard::is_locked(&doc_path));
        }
        let after_drop = (lock_path.exists(), LockGuard::is_locked(&doc_path));

        assert_eq!(
            (initially_locked, while_held, after_drop),
            (false, (true, true), (false, false)),
        );
    }

    #[test]
    fn lock_guard_does_not_replace_an_existing_lock() {
        let temp_dir = TempDir::new().unwrap();
        let doc_path = temp_dir.path().join("test.md");
        fs::write(&doc_path, "content").unwrap();

        let lock_path = LockGuard::lock_path(&doc_path);
        fs::write(&lock_path, "another review").unwrap();
        let acquired = LockGuard::acquire(&doc_path).is_ok();
        let contents = fs::read_to_string(lock_path).unwrap();

        assert_eq!((acquired, contents), (false, "another review".into()));
    }

    #[test]
    fn lock_guard_does_not_remove_a_replacement_lock() {
        let temp_dir = TempDir::new().unwrap();
        let doc_path = temp_dir.path().join("test.md");
        fs::write(&doc_path, "content").unwrap();

        let old_guard = LockGuard::acquire(&doc_path).unwrap();
        fs::remove_file(LockGuard::lock_path(&doc_path)).unwrap();
        let new_guard = LockGuard::acquire(&doc_path).unwrap();
        drop(old_guard);
        let replacement_remains = LockGuard::is_locked(&doc_path);
        drop(new_guard);

        assert_eq!(
            (replacement_remains, LockGuard::is_locked(&doc_path)),
            (true, false),
        );
    }

    #[test]
    fn lock_guard_disarm_prevents_cleanup() {
        let temp_dir = TempDir::new().unwrap();
        let doc_path = temp_dir.path().join("test.md");
        fs::write(&doc_path, "content").unwrap();

        let lock_path = LockGuard::lock_path(&doc_path);

        let while_held;
        {
            let mut guard = LockGuard::acquire(&doc_path).unwrap();
            while_held = lock_path.exists();
            guard.disarm();
        }

        assert_eq!((while_held, lock_path.exists()), (true, true));
    }

    #[test]
    fn cleanup_guard_removes_lock_file() {
        let temp_dir = TempDir::new().unwrap();
        let doc_path = temp_dir.path().join("test.md");
        fs::write(&doc_path, "content").unwrap();

        let lock_path = LockGuard::lock_path(&doc_path);
        fs::write(&lock_path, "").unwrap();

        let existed_before_cleanup = lock_path.exists();

        {
            let _guard = CleanupGuard::new(&doc_path, None);
        }

        // Lock file should be removed when cleanup guard is dropped
        let exists_after_cleanup = lock_path.exists();
        assert_eq!(
            (existed_before_cleanup, exists_after_cleanup),
            (true, false)
        );
    }
}
