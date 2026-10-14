use std::collections::hash_map::DefaultHasher;
use std::fs::{File, OpenOptions, TryLockError};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};

/// Serializes armyknife launches that would otherwise match the same cwd-only
/// `thread/started` notification.
pub fn acquire_launch_lock(cwd: &Path) -> anyhow::Result<File> {
    let (file, lock_path) = open_launch_lock(cwd)?;

    file.lock()
        .with_context(|| format!("failed to lock Codex launch {}", lock_path.display()))?;
    Ok(file)
}

/// Acquires the per-cwd launch lock before `timeout` elapses.
pub fn acquire_launch_lock_with_timeout(cwd: &Path, timeout: Duration) -> anyhow::Result<File> {
    let (file, lock_path) = open_launch_lock(cwd)?;
    let deadline = Instant::now()
        .checked_add(timeout)
        .context("Codex launch-lock timeout is out of range")?;

    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(TryLockError::WouldBlock) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(anyhow!(
                        "timed out waiting for another Codex launch in the same directory"
                    ));
                }
                std::thread::sleep(remaining.min(Duration::from_millis(50)));
            }
            Err(TryLockError::Error(error)) => {
                return Err(error).with_context(|| {
                    format!("failed to lock Codex launch {}", lock_path.display())
                });
            }
        }
    }
}

fn open_launch_lock(cwd: &Path) -> anyhow::Result<(File, PathBuf)> {
    let canonical_cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let mut hasher = DefaultHasher::new();
    canonical_cwd.hash(&mut hasher);
    let lock_dir = crate::shared::dirs::cache_dir()
        .context("could not determine cache directory for Codex launch lock")?
        .join("armyknife")
        .join("codex-launch-locks");
    std::fs::create_dir_all(&lock_dir).context("failed to create Codex launch lock directory")?;
    let lock_path = lock_dir.join(format!("{:016x}.lock", hasher.finish()));
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("failed to open Codex launch lock {}", lock_path.display()))?;

    Ok((file, lock_path))
}
