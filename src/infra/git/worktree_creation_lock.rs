use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;

use anyhow::{Context, Result};

use super::GitRepo;

const LOCK_FILE_NAME: &str = "armyknife-worktree-creation.lock";

pub(crate) struct WorktreeCreationLock {
    file: File,
}

impl WorktreeCreationLock {
    pub(crate) fn acquire(repo: &GitRepo) -> Result<Self> {
        let lock_path = repo.common_dir().join(LOCK_FILE_NAME);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| {
                format!(
                    "Failed to open worktree creation lock at {}",
                    lock_path.display()
                )
            })?;

        // SAFETY: `file` remains open until the lock guard is dropped.
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if result != 0 {
            return Err(anyhow::anyhow!(
                "Failed to acquire worktree creation lock: {}",
                std::io::Error::last_os_error()
            ));
        }

        Ok(Self { file })
    }
}

impl Drop for WorktreeCreationLock {
    fn drop(&mut self) {
        // SAFETY: `file` remains open until this guard is dropped.
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
