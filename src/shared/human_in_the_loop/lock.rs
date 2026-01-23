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
