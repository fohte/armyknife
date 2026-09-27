use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::session_rows::SessionTask;
use super::tq_snapshot::TqSnapshot;
use crate::shared::cache;

fn path() -> Option<PathBuf> {
    cache::base_dir().map(|dir| dir.join("agent").join("watch-tq-session-tasks.json"))
}

pub(super) fn load() -> Result<Option<TqSnapshot>> {
    let Some(path) = path() else {
        return Ok(None);
    };
    load_from(&path)
}

fn load_from(path: &Path) -> Result<Option<TqSnapshot>> {
    match fs::read(path) {
        Ok(content) => {
            let cache: CachedSnapshot = serde_json::from_slice(&content)
                .with_context(|| format!("failed to parse cache file: {}", path.display()))?;
            let snapshot = match cache {
                CachedSnapshot::Snapshot(snapshot) => snapshot,
                CachedSnapshot::Legacy(session_tasks) => {
                    let fetched_at = fs::metadata(path)
                        .and_then(|metadata| metadata.modified())
                        .map(DateTime::<Utc>::from)
                        .unwrap_or_else(|_| Utc::now());
                    TqSnapshot::from_legacy(session_tasks, fetched_at)
                }
            };
            Ok(Some(snapshot))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => {
            Err(error).with_context(|| format!("failed to read cache file: {}", path.display()))
        }
    }
}

pub(super) fn store(snapshot: &TqSnapshot) -> Result<()> {
    let path = path().context("cache directory is unavailable")?;
    store_to(&path, snapshot)
}

fn store_to(path: &Path, snapshot: &TqSnapshot) -> Result<()> {
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
    serde_json::to_writer(&mut temp_file, snapshot)
        .context("failed to serialize tq sidebar snapshot")?;
    temp_file.write_all(b"\n")?;
    temp_file.as_file().sync_all()?;
    temp_file
        .persist(path)
        .with_context(|| format!("failed to replace cache file: {}", path.display()))?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(untagged)]
enum CachedSnapshot {
    Snapshot(TqSnapshot),
    Legacy(HashMap<String, SessionTask>),
}
