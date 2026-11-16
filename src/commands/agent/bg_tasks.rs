use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::{bg_storage, store};
use crate::infra::process;
use crate::shared::cache;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Covers the interval between task registration and the worker recording its PID.
const UNREGISTERED_WORKER_GRACE: Duration = Duration::from_secs(5 * 60);

#[derive(Deserialize, Serialize)]
struct TaskRecord {
    parent_pid: Option<u32>,
    worker_pid: Option<u32>,
    command_pid: Option<u32>,
    #[serde(default)]
    stdout_path: Option<PathBuf>,
    #[serde(default)]
    description: Option<String>,
}

pub(crate) fn tasks_dir() -> Result<PathBuf> {
    let base_dir = cache::base_dir().context("Unable to determine cache directory")?;
    Ok(tasks_dir_in(&base_dir))
}

fn tasks_dir_in(base_dir: &Path) -> PathBuf {
    base_dir.join("agent-bg").join("tasks")
}

fn session_tasks_dir(tasks_root: &Path, session_id: &str) -> PathBuf {
    bg_storage::session_dir(tasks_root, session_id)
}

fn marker_path(tasks_root: &Path, session_id: &str, task_id: &str) -> Result<PathBuf> {
    let task_id = bg_storage::task_id(task_id)?;
    Ok(session_tasks_dir(tasks_root, session_id).join(format!("{task_id}.pending")))
}

pub(crate) fn register(
    session_id: &str,
    task_id: &str,
    description: &str,
    stdout_path: &Path,
) -> Result<()> {
    let tasks_root = tasks_dir()?;
    register_in(&tasks_root, session_id, task_id, description, stdout_path)?;
    if let Err(error) = store::touch_session(session_id) {
        let _ = clear_in(&tasks_root, session_id, task_id);
        return Err(error).context("failed to refresh session after registering background task");
    }
    Ok(())
}

fn register_in(
    tasks_root: &Path,
    session_id: &str,
    task_id: &str,
    description: &str,
    stdout_path: &Path,
) -> Result<()> {
    let marker = marker_path(tasks_root, session_id, task_id)?;
    let dir = marker
        .parent()
        .context("background task marker has no parent directory")?;
    bg_storage::create_dir_secure(dir).with_context(|| {
        format!(
            "failed to create background task directory: {}",
            dir.display()
        )
    })?;

    with_registry_lock(tasks_root, || {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&marker)
            .with_context(|| format!("failed to register background task: {}", marker.display()))?;
        serde_json::to_writer(
            &mut file,
            &TaskRecord {
                parent_pid: Some(std::process::id()),
                worker_pid: None,
                command_pid: None,
                stdout_path: Some(stdout_path.to_path_buf()),
                description: Some(description.to_owned()),
            },
        )?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        Ok(())
    })
}

pub(crate) fn set_worker_pid(session_id: &str, task_id: &str, pid: u32) -> Result<()> {
    update_record(&tasks_dir()?, session_id, task_id, |record| {
        record.parent_pid = None;
        record.worker_pid = Some(pid);
    })
}

pub(crate) fn set_command_pid(session_id: &str, task_id: &str, pid: u32) -> Result<()> {
    update_record(&tasks_dir()?, session_id, task_id, |record| {
        record.parent_pid = None;
        record.command_pid = Some(pid);
    })
}

fn update_record(
    tasks_root: &Path,
    session_id: &str,
    task_id: &str,
    update: impl FnOnce(&mut TaskRecord),
) -> Result<()> {
    let marker = marker_path(tasks_root, session_id, task_id)?;
    with_registry_lock(tasks_root, || {
        let content = match fs::read_to_string(&marker) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read task marker: {}", marker.display()));
            }
        };
        let mut record: TaskRecord = serde_json::from_str(&content)
            .with_context(|| format!("failed to parse task marker: {}", marker.display()))?;
        update(&mut record);
        write_record(&marker, &record)
    })
}

fn write_record(marker: &Path, record: &TaskRecord) -> Result<()> {
    bg_storage::write_atomic(marker, |file| {
        serde_json::to_writer(&mut *file, record)?;
        file.write_all(b"\n")?;
        Ok(())
    })
}

pub(crate) fn has_pending_in(tasks_root: &Path, session_id: &str) -> Result<bool> {
    let scan = scan_pending_in(tasks_root, session_id)?;
    if scan.stale_cleared {
        store::touch_session(session_id)?;
    }
    Ok(scan.pending)
}

pub(crate) struct PendingScan {
    pub pending: bool,
    pub stale_cleared: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingTask {
    pub id: String,
    pub description: String,
    pub elapsed_seconds: u64,
    pub stdout_path: PathBuf,
}

pub(crate) fn pending_tasks(session_id: &str) -> Result<Vec<PendingTask>> {
    let tasks_root = tasks_dir()?;
    let (tasks, stale_cleared) = pending_tasks_in(&tasks_root, session_id)?;
    if stale_cleared {
        store::touch_session(session_id)?;
    }
    Ok(tasks)
}

fn pending_tasks_in(tasks_root: &Path, session_id: &str) -> Result<(Vec<PendingTask>, bool)> {
    if !tasks_root.exists() {
        return Ok((Vec::new(), false));
    }
    with_registry_lock(tasks_root, || {
        let dir = session_tasks_dir(tasks_root, session_id);
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok((Vec::new(), false));
            }
            Err(error) => {
                return Err(error).with_context(|| format!("failed to read {}", dir.display()));
            }
        };

        let mut tasks = Vec::new();
        let mut stale_cleared = false;
        for entry in entries {
            let entry =
                entry.with_context(|| format!("failed to read an entry in {}", dir.display()))?;
            let path = entry.path();
            if !entry.file_type()?.is_file()
                || !path
                    .extension()
                    .is_some_and(|extension| extension == "pending")
            {
                continue;
            }

            match marker_is_pending(&path)? {
                MarkerState::Pending(record) => {
                    let Some(task_id) = path.file_stem().and_then(|stem| stem.to_str()) else {
                        continue;
                    };
                    bg_storage::task_id(task_id)?;
                    let elapsed_seconds = fs::metadata(&path)
                        .ok()
                        .and_then(|metadata| metadata.modified().ok())
                        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                        .unwrap_or_default()
                        .as_secs();
                    tasks.push(PendingTask {
                        id: task_id.to_owned(),
                        description: record
                            .as_ref()
                            .and_then(|record| record.description.clone())
                            .unwrap_or_else(|| "Background command".to_owned()),
                        elapsed_seconds,
                        stdout_path: record
                            .and_then(|record| record.stdout_path)
                            .unwrap_or_else(|| super::bg::stdout_path(task_id)),
                    });
                }
                MarkerState::ClearedStale => stale_cleared = true,
                MarkerState::NotPending => {}
            }
        }
        Ok((tasks, stale_cleared))
    })
}

pub(crate) fn scan_pending_in(tasks_root: &Path, session_id: &str) -> Result<PendingScan> {
    let (tasks, stale_cleared) = pending_tasks_in(tasks_root, session_id)?;
    Ok(PendingScan {
        pending: !tasks.is_empty(),
        stale_cleared,
    })
}

enum MarkerState {
    Pending(Option<TaskRecord>),
    NotPending,
    ClearedStale,
}

fn marker_is_pending(marker: &Path) -> Result<MarkerState> {
    let metadata = match fs::metadata(marker) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(MarkerState::NotPending);
        }
        Err(error) => {
            return Err(error).with_context(|| format!("failed to stat {}", marker.display()));
        }
    };
    let age = metadata
        .modified()
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .unwrap_or_default();
    let content = match fs::read_to_string(marker) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(MarkerState::NotPending);
        }
        Err(_error) if age < UNREGISTERED_WORKER_GRACE => return Ok(MarkerState::Pending(None)),
        Err(error) => {
            tracing::warn!(
                event = "agent.bg_run.task_marker_read_failed",
                marker = %marker.display(),
                error = %error,
            );
            fs::remove_file(marker)?;
            return Ok(MarkerState::ClearedStale);
        }
    };
    let record = match serde_json::from_str::<TaskRecord>(&content) {
        Ok(record) => record,
        Err(_error) if age < UNREGISTERED_WORKER_GRACE => return Ok(MarkerState::Pending(None)),
        Err(error) => {
            tracing::warn!(
                event = "agent.bg_run.task_marker_invalid",
                marker = %marker.display(),
                error = %error,
            );
            fs::remove_file(marker)?;
            return Ok(MarkerState::ClearedStale);
        }
    };

    let pids = [record.parent_pid, record.worker_pid, record.command_pid];
    if pids.into_iter().flatten().any(process::is_process_alive) {
        return Ok(MarkerState::Pending(Some(record)));
    }
    if age < UNREGISTERED_WORKER_GRACE {
        return Ok(MarkerState::Pending(Some(record)));
    }

    tracing::info!(
        event = "agent.bg_run.stale_task_cleared",
        marker = %marker.display(),
    );
    fs::remove_file(marker)?;
    Ok(MarkerState::ClearedStale)
}

pub(crate) fn clear(session_id: &str, task_id: &str) -> Result<()> {
    clear_in(&tasks_dir()?, session_id, task_id)?;
    store::touch_session(session_id)
}

pub(crate) fn clear_best_effort(session_id: &str, task_id: &str) {
    if let Err(error) = clear(session_id, task_id) {
        tracing::warn!(
            event = "agent.bg_run.task_clear_failed",
            session = %session_id,
            task = %task_id,
            error = %error,
        );
    }
}

fn clear_in(tasks_root: &Path, session_id: &str, task_id: &str) -> Result<()> {
    let marker = marker_path(tasks_root, session_id, task_id)?;
    with_registry_lock(tasks_root, || match fs::remove_file(&marker) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("failed to clear background task: {}", marker.display())),
    })
}

fn with_registry_lock<T>(tasks_root: &Path, operation: impl FnOnce() -> Result<T>) -> Result<T> {
    bg_storage::create_dir_secure(tasks_root).with_context(|| {
        format!(
            "failed to create background task registry: {}",
            tasks_root.display()
        )
    })?;
    let lock_path = tasks_root.join(".lock");
    let lock_file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&lock_path)
        .with_context(|| format!("failed to open task registry lock: {}", lock_path.display()))?;
    lock_file
        .lock()
        .with_context(|| format!("failed to lock task registry: {}", lock_path.display()))?;
    operation()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;
    use tempfile::TempDir;

    use super::{PendingTask, pending_tasks_in, register_in};
    use crate::shared::hex;

    #[test]
    fn pending_task_listing_returns_registered_task_details() {
        let tasks_root = TempDir::new().expect("task registry directory should be created");
        let session_id = "fixture-session";
        let task_id = uuid::Uuid::new_v4().to_string();
        let stdout_path = PathBuf::from("/fixture/background.stdout");
        register_in(
            tasks_root.path(),
            session_id,
            &task_id,
            "Run synthetic task",
            &stdout_path,
        )
        .expect("background task should be registered");

        let (mut tasks, stale_cleared) =
            pending_tasks_in(tasks_root.path(), session_id).expect("pending tasks should be read");
        for task in &mut tasks {
            task.elapsed_seconds = 0;
        }

        assert_eq!(
            (tasks, stale_cleared),
            (
                vec![PendingTask {
                    id: task_id.clone(),
                    description: "Run synthetic task".to_owned(),
                    elapsed_seconds: 0,
                    stdout_path,
                }],
                false,
            ),
        );
    }

    #[test]
    fn pending_task_listing_reads_legacy_task_markers() {
        let tasks_root = TempDir::new().expect("task registry directory should be created");
        let session_id = "fixture-session";
        let task_id = uuid::Uuid::new_v4().to_string();
        let session_directory = tasks_root.path().join(hex::encode(session_id.as_bytes()));
        std::fs::create_dir_all(&session_directory)
            .expect("task registry session directory should be created");
        std::fs::write(
            session_directory.join(format!("{task_id}.pending")),
            serde_json::to_vec(&json!({
                "parent_pid": std::process::id(),
                "worker_pid": null,
                "command_pid": null,
            }))
            .expect("legacy task marker should serialize"),
        )
        .expect("legacy task marker should be written");

        let (mut tasks, stale_cleared) =
            pending_tasks_in(tasks_root.path(), session_id).expect("pending tasks should be read");
        for task in &mut tasks {
            task.elapsed_seconds = 0;
        }

        assert_eq!(
            (tasks, stale_cleared),
            (
                vec![PendingTask {
                    id: task_id.clone(),
                    description: "Background command".to_owned(),
                    elapsed_seconds: 0,
                    stdout_path: super::super::bg::stdout_path(&task_id),
                }],
                false,
            ),
        );
    }
}
