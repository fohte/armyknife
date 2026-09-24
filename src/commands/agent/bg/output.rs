use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub(super) fn output_dir() -> Result<PathBuf> {
    let dir = std::env::temp_dir()
        .join("armyknife")
        .join("agent-bg")
        .join("output");
    create_dir_secure(&dir)
        .with_context(|| format!("failed to create output directory: {}", dir.display()))?;
    Ok(dir)
}

pub(super) fn open_output_file(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    let mut permissions = file.metadata()?.permissions();
    if permissions.mode() & 0o077 != 0 {
        permissions.set_mode(0o600);
        file.set_permissions(permissions)?;
    }
    Ok(file)
}

pub(super) fn create_output_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

fn create_dir_secure(dir: &Path) -> io::Result<()> {
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
