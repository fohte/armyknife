use std::fs::{self, File, OpenOptions};
use std::path::Path;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

pub(super) fn lock_resolution_file(
    sessions_dir: &Path,
    lock_directory: &str,
    key: &str,
) -> Result<File> {
    let key_hash = Sha256::digest(key.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join("");
    lock_file(sessions_dir, lock_directory, &format!("{key_hash}.lock"))
}

pub(super) fn lock_file(
    sessions_dir: &Path,
    lock_directory: &str,
    file_name: &str,
) -> Result<File> {
    let lock_dir = sessions_dir.join(lock_directory);
    fs::create_dir_all(&lock_dir)
        .with_context(|| format!("creating lock directory {}", lock_dir.display()))?;
    // Stable lock files avoid a deletion race between waiting clients.
    let lock_path = lock_dir.join(file_name);
    let lock_file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("opening lock file {}", lock_path.display()))?;
    lock_file
        .lock()
        .with_context(|| format!("locking file {}", lock_path.display()))?;
    Ok(lock_file)
}
