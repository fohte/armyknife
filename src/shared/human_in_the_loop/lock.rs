use std::fs;
use std::path::{Path, PathBuf};

use super::error::Result;
use crate::infra::tmux;

/// RAII guard for lock file cleanup in the review launcher.
///
/// When WezTerm fails to launch, this guard ensures the lock file is removed.
/// When WezTerm launches successfully, call `disarm()` to prevent cleanup
/// (the review-complete process will handle it).
pub struct LockGuard {
    lock_path: PathBuf,
    disarmed: bool,
}

impl LockGuard {
    /// Create a lock file and return a guard.
    pub fn acquire(document_path: &Path) -> Result<Self> {
        let lock_path = Self::lock_path(document_path);
        fs::write(&lock_path, "")?;
        Ok(Self {
            lock_path,
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
    ///
    /// Call this after WezTerm launches successfully, since the review-complete
    /// process will handle lock cleanup.
    pub fn disarm(&mut self) {
        self.disarmed = true;
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if !self.disarmed {
            let _ = fs::remove_file(&self.lock_path);
        }
    }
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

        // Restore tmux session
        if let Some(ref target) = self.tmux_target {
            let _ = tmux::switch_to_session(target);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use tempfile::TempDir;

    #[rstest]
    #[case("file.md", "file.md.lock")]
    #[case("file.txt", "file.txt.lock")]
    #[case("file", "file.lock")]
    #[case("file.tar.gz", "file.tar.gz.lock")]
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

        // Lock file should not exist initially
        assert!(!lock_path.exists());

        {
            let _guard = LockGuard::acquire(&doc_path).unwrap();
            // Lock file should exist while guard is held
            assert!(lock_path.exists());
            assert!(LockGuard::is_locked(&doc_path));
        }

        // Lock file should be removed when guard is dropped
        assert!(!lock_path.exists());
        assert!(!LockGuard::is_locked(&doc_path));
    }

    #[test]
    fn lock_guard_disarm_prevents_cleanup() {
        let temp_dir = TempDir::new().unwrap();
        let doc_path = temp_dir.path().join("test.md");
        fs::write(&doc_path, "content").unwrap();

        let lock_path = LockGuard::lock_path(&doc_path);

        {
            let mut guard = LockGuard::acquire(&doc_path).unwrap();
            assert!(lock_path.exists());
            guard.disarm();
        }

        // Lock file should still exist after disarmed guard is dropped
        assert!(lock_path.exists());

        // Cleanup
        fs::remove_file(&lock_path).unwrap();
    }

    #[test]
    fn cleanup_guard_removes_lock_file() {
        let temp_dir = TempDir::new().unwrap();
        let doc_path = temp_dir.path().join("test.md");
        fs::write(&doc_path, "content").unwrap();

        let lock_path = LockGuard::lock_path(&doc_path);
        fs::write(&lock_path, "").unwrap();

        assert!(lock_path.exists());

        {
            let _guard = CleanupGuard::new(&doc_path, None);
        }

        // Lock file should be removed when cleanup guard is dropped
        assert!(!lock_path.exists());
    }
}
