use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::store;
use crate::shared::{cache, hex};

/// Covers the interval between task registration and the worker recording its PID.
const UNREGISTERED_WORKER_GRACE: Duration = Duration::from_secs(5 * 60);

#[derive(Deserialize, Serialize)]
struct TaskRecord {
    parent_pid: Option<u32>,
    worker_pid: Option<u32>,
    command_pid: Option<u32>,
}

pub(crate) fn tasks_dir() -> Result<PathBuf> {
    let base_dir = cache::base_dir().context("Unable to determine cache directory")?;
    Ok(tasks_dir_in(&base_dir))
}

fn tasks_dir_in(base_dir: &Path) -> PathBuf {
    base_dir.join("agent-bg").join("tasks")
}

fn session_tasks_dir(tasks_root: &Path, session_id: &str) -> PathBuf {
    tasks_root.join(hex::encode(session_id.as_bytes()))
}

fn marker_path(tasks_root: &Path, session_id: &str, task_id: &str) -> Result<PathBuf> {
    let task_id = Uuid::parse_str(task_id).context("invalid background task ID")?;
    Ok(session_tasks_dir(tasks_root, session_id).join(format!("{task_id}.pending")))
}

pub(crate) fn register(session_id: &str, task_id: &str) -> Result<()> {
    let tasks_root = tasks_dir()?;
    register_in(&tasks_root, session_id, task_id)?;
    if let Err(error) = store::touch_session(session_id) {
        let _ = clear_in(&tasks_root, session_id, task_id);
        return Err(error).context("failed to refresh session after registering background task");
    }
    Ok(())
}

fn register_in(tasks_root: &Path, session_id: &str, task_id: &str) -> Result<()> {
    let marker = marker_path(tasks_root, session_id, task_id)?;
    let dir = marker
        .parent()
        .context("background task marker has no parent directory")?;
    create_dir_secure(dir).with_context(|| {
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
    let temp_path = marker.with_extension(format!("tmp.{}", Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp_path)
        .with_context(|| {
            format!(
                "failed to create task marker temp file: {}",
                temp_path.display()
            )
        })?;
    serde_json::to_writer(&mut file, record)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::rename(&temp_path, marker)
        .with_context(|| format!("failed to update task marker: {}", marker.display()))?;
    Ok(())
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

pub(crate) fn scan_pending_in(tasks_root: &Path, session_id: &str) -> Result<PendingScan> {
    if !tasks_root.exists() {
        return Ok(PendingScan {
            pending: false,
            stale_cleared: false,
        });
    }
    with_registry_lock(tasks_root, || {
        let dir = session_tasks_dir(tasks_root, session_id);
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(PendingScan {
                    pending: false,
                    stale_cleared: false,
                });
            }
            Err(error) => {
                return Err(error).with_context(|| format!("failed to read {}", dir.display()));
            }
        };

        let mut stale_cleared = false;
        for entry in entries {
            let entry =
                entry.with_context(|| format!("failed to read an entry in {}", dir.display()))?;
            let path = entry.path();
            if entry.file_type()?.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "pending")
            {
                match marker_is_pending(&path)? {
                    MarkerState::Pending => {
                        return Ok(PendingScan {
                            pending: true,
                            stale_cleared,
                        });
                    }
                    MarkerState::ClearedStale => stale_cleared = true,
                    MarkerState::NotPending => {}
                }
            }
        }
        Ok(PendingScan {
            pending: false,
            stale_cleared,
        })
    })
}

enum MarkerState {
    Pending,
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
        Err(_error) if age < UNREGISTERED_WORKER_GRACE => return Ok(MarkerState::Pending),
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
        Err(_error) if age < UNREGISTERED_WORKER_GRACE => return Ok(MarkerState::Pending),
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
    if pids.into_iter().flatten().any(process_is_alive) {
        return Ok(MarkerState::Pending);
    }
    if age < UNREGISTERED_WORKER_GRACE {
        return Ok(MarkerState::Pending);
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
    create_dir_secure(tasks_root).with_context(|| {
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

fn create_dir_secure(dir: &Path) -> std::io::Result<()> {
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

fn process_is_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    // SAFETY: signal 0 only checks whether this process ID exists.
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}
