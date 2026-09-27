use std::fs::File;
use std::path::Path;

use anyhow::Context;

use super::GitRepo;
use super::error::Result;
use super::file_lock::{lock_exclusive, open_lock_file, try_lock_exclusive, unlock};

const LOCK_FILE_NAME: &str = "armyknife-worktree-creation.lock";

pub(crate) struct WorktreeCreationLock {
    file: File,
}

impl WorktreeCreationLock {
    pub(crate) fn acquire(repo: &GitRepo) -> Result<Self> {
        let lock_path = repo.common_dir().join(LOCK_FILE_NAME);
        Self::acquire_at(&lock_path)
    }

    fn acquire_at(lock_path: &Path) -> Result<Self> {
        let file = open_lock_file(lock_path, "worktree creation")?;
        if !try_lock_exclusive(&file).with_context(|| {
            format!(
                "Failed to acquire worktree creation lock at {}",
                lock_path.display()
            )
        })? {
            eprintln!("Waiting for another worktree creation in this repository to finish...");
            lock_exclusive(&file, "worktree creation").with_context(|| {
                format!(
                    "Failed to acquire worktree creation lock at {}",
                    lock_path.display()
                )
            })?;
        }

        Ok(Self { file })
    }
}

impl Drop for WorktreeCreationLock {
    fn drop(&mut self) {
        unlock(&self.file);
    }
}
