use std::fs::{self, OpenOptions};
use std::path::PathBuf;

use anyhow::{Context, Result};
use uuid::Uuid;

use crate::shared::cache;

fn session_tasks_dir(session_id: &str) -> Result<PathBuf> {
    let session_key = session_id
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let base_dir = cache::base_dir().context("Unable to determine cache directory")?;
    Ok(base_dir.join("agent-bg").join("tasks").join(session_key))
}

fn marker_path(session_id: &str, task_id: &str) -> Result<PathBuf> {
    let task_id = Uuid::parse_str(task_id).context("invalid background task ID")?;
    Ok(session_tasks_dir(session_id)?.join(format!("{task_id}.pending")))
}

pub(crate) fn register(session_id: &str, task_id: &str) -> Result<()> {
    let marker = marker_path(session_id, task_id)?;
    let dir = marker
        .parent()
        .context("background task marker has no parent directory")?;
    fs::create_dir_all(dir).with_context(|| {
        format!(
            "failed to create background task directory: {}",
            dir.display()
        )
    })?;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker)
        .with_context(|| format!("failed to register background task: {}", marker.display()))?;
    Ok(())
}

pub(crate) fn has_pending(session_id: &str) -> Result<bool> {
    let dir = session_tasks_dir(session_id)?;
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", dir.display()));
        }
    };

    for entry in entries {
        let entry =
            entry.with_context(|| format!("failed to read an entry in {}", dir.display()))?;
        if entry.file_type()?.is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "pending")
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn clear(session_id: &str, task_id: &str) -> Result<()> {
    let marker = marker_path(session_id, task_id)?;
    match fs::remove_file(&marker) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("failed to clear background task: {}", marker.display())),
    }
}
