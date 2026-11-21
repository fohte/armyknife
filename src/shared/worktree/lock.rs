use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

#[derive(Debug)]
pub(super) enum WorktreeLockError {
    Busy,
    Io(anyhow::Error),
}

pub(super) fn default_lock_dir() -> Result<PathBuf> {
    crate::shared::cache::base_dir()
        .map(|path| path.join("worktree-locks"))
        .ok_or_else(|| anyhow::anyhow!("Could not determine cache directory for worktree locks"))
}

pub(super) fn try_lock_shared(
    lock_dir: &Path,
    worktree_path: &Path,
) -> std::result::Result<File, WorktreeLockError> {
    let file = open_lock_file(lock_dir, worktree_path, "lock").map_err(WorktreeLockError::Io)?;
    match file.try_lock_shared() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => Err(WorktreeLockError::Busy),
        Err(std::fs::TryLockError::Error(error)) => Err(WorktreeLockError::Io(error.into())),
    }
}

pub(super) fn try_lock_deletion(
    lock_dir: &Path,
    worktree_path: &Path,
) -> std::result::Result<File, WorktreeLockError> {
    let file =
        open_lock_file(lock_dir, worktree_path, "deletion.lock").map_err(WorktreeLockError::Io)?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => Err(WorktreeLockError::Busy),
        Err(std::fs::TryLockError::Error(error)) => Err(WorktreeLockError::Io(error.into())),
    }
}

pub(super) fn lock_exclusive(lock_dir: &Path, worktree_path: &Path) -> Result<File> {
    let file = open_lock_file(lock_dir, worktree_path, "lock")?;
    file.lock()
        .with_context(|| format!("locking worktree for deletion: {}", worktree_path.display()))?;
    Ok(file)
}

fn open_lock_file(lock_dir: &Path, worktree_path: &Path, suffix: &str) -> Result<File> {
    fs::create_dir_all(lock_dir)
        .with_context(|| format!("creating worktree lock directory {}", lock_dir.display()))?;
    let canonical_path = canonicalize_with_missing_suffix(worktree_path)?;
    let key_hash = Sha256::digest(canonical_path.as_os_str().as_encoded_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join("");
    let lock_path = lock_dir.join(format!("{key_hash}.{suffix}"));
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("opening worktree lock {}", lock_path.display()))
}

fn canonicalize_with_missing_suffix(path: &Path) -> Result<PathBuf> {
    let absolute_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut existing = absolute_path.as_path();
    let mut missing_suffix = Vec::new();

    loop {
        if let Ok(mut canonical_path) = existing.canonicalize() {
            for component in missing_suffix.into_iter().rev() {
                canonical_path.push(component);
            }
            return Ok(canonical_path);
        }

        let Some(file_name) = existing.file_name() else {
            return Err(anyhow::anyhow!(
                "could not resolve worktree path {}",
                path.display()
            ));
        };
        missing_suffix.push(file_name.to_os_string());
        existing = existing
            .parent()
            .ok_or_else(|| anyhow::anyhow!("could not resolve worktree path {}", path.display()))?;
    }
}
