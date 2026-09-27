use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::session_rows::SessionTask;
use crate::shared::cache;

fn path() -> Option<PathBuf> {
    cache::base_dir().map(|dir| dir.join("agent").join("watch-tq-session-tasks.json"))
}

pub(super) fn load() -> Result<HashMap<String, SessionTask>> {
    let Some(path) = path() else {
        return Ok(HashMap::new());
    };
    load_from(&path)
}

fn load_from(path: &Path) -> Result<HashMap<String, SessionTask>> {
    match fs::read(path) {
        Ok(content) => serde_json::from_slice(&content)
            .with_context(|| format!("failed to parse cache file: {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
        Err(error) => {
            Err(error).with_context(|| format!("failed to read cache file: {}", path.display()))
        }
    }
}

pub(super) fn store(task_by_session: &HashMap<String, SessionTask>) -> Result<()> {
    let path = path().context("cache directory is unavailable")?;
    store_to(&path, task_by_session)
}

fn store_to(path: &Path, task_by_session: &HashMap<String, SessionTask>) -> Result<()> {
    let parent = path
        .parent()
        .context("cache file path has no parent directory")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create cache directory: {}", parent.display()))?;

    let mut temp_file = tempfile::NamedTempFile::new_in(parent).with_context(|| {
        format!(
            "failed to create temporary cache file in {}",
            parent.display()
        )
    })?;
    serde_json::to_writer(&mut temp_file, task_by_session)
        .context("failed to serialize tq session-task cache")?;
    temp_file.write_all(b"\n")?;
    temp_file.as_file().sync_all()?;
    temp_file
        .persist(path)
        .with_context(|| format!("failed to replace cache file: {}", path.display()))?;
    Ok(())
}
