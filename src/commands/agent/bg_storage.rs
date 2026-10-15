use std::fs::{self, File, OpenOptions};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use uuid::Uuid;

use crate::shared::hex;

pub(super) fn session_dir(root: &Path, session_id: &str) -> PathBuf {
    root.join(hex::encode(session_id.as_bytes()))
}

pub(super) fn task_id(task_id: &str) -> Result<Uuid> {
    Uuid::parse_str(task_id).context("invalid background task ID")
}

pub(super) fn create_dir_secure(dir: &Path) -> std::io::Result<()> {
    if !dir.exists() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        return Ok(());
    }

    let metadata = fs::metadata(dir)?;
    let mut permissions = metadata.permissions();
    if permissions.mode() & 0o077 != 0 {
        permissions.set_mode(0o700);
        fs::set_permissions(dir, permissions)?;
    }
    Ok(())
}

pub(super) fn write_atomic(
    path: &Path,
    write_content: impl FnOnce(&mut File) -> Result<()>,
) -> Result<()> {
    let parent = path
        .parent()
        .context("background storage path has no parent directory")?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("background storage path has no file name")?;
    let temp_path = parent.join(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp_path)
            .with_context(|| {
                format!(
                    "failed to create temporary background storage file: {}",
                    temp_path.display()
                )
            })?;
        write_content(&mut file)?;
        file.sync_all()?;
        fs::rename(&temp_path, path).with_context(|| {
            format!("failed to save background storage file: {}", path.display())
        })?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}
