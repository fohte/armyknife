use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::session_status;
use super::types::{BG_RUN_PENDING_TASK_MARKER, Session};
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
    if let Err(error) = session_status::touch_session(session_id) {
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
    has_pending_with_touch_in(tasks_root, session_id, true)
}

fn has_pending_for_hook_in(tasks_root: &Path, session_id: &str) -> Result<bool> {
    has_pending_with_touch_in(tasks_root, session_id, false)
}

fn has_pending_with_touch_in(
    tasks_root: &Path,
    session_id: &str,
    touch_session: bool,
) -> Result<bool> {
    if !tasks_root.exists() {
        return Ok(false);
    }
    let (pending, stale_cleared) = with_registry_lock(tasks_root, || {
        let dir = session_tasks_dir(tasks_root, session_id);
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok((false, false));
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
                let (pending, cleared) = marker_is_pending(&path)?;
                stale_cleared |= cleared;
                if pending {
                    return Ok((true, stale_cleared));
                }
            }
        }
        Ok((false, stale_cleared))
    })?;
    if stale_cleared && touch_session {
        session_status::touch_session(session_id)?;
    }
    Ok(pending)
}

fn marker_is_pending(marker: &Path) -> Result<(bool, bool)> {
    let metadata = match fs::metadata(marker) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((false, false)),
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
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((false, false)),
        Err(_error) if age < UNREGISTERED_WORKER_GRACE => return Ok((true, false)),
        Err(error) => {
            tracing::warn!(
                event = "agent.bg_run.task_marker_read_failed",
                marker = %marker.display(),
                error = %error,
            );
            fs::remove_file(marker)?;
            return Ok((false, true));
        }
    };
    let record = match serde_json::from_str::<TaskRecord>(&content) {
        Ok(record) => record,
        Err(_error) if age < UNREGISTERED_WORKER_GRACE => return Ok((true, false)),
        Err(error) => {
            tracing::warn!(
                event = "agent.bg_run.task_marker_invalid",
                marker = %marker.display(),
                error = %error,
            );
            fs::remove_file(marker)?;
            return Ok((false, true));
        }
    };

    let pids = [record.parent_pid, record.worker_pid, record.command_pid];
    if pids.into_iter().flatten().any(process_is_alive) {
        return Ok((true, false));
    }
    if age < UNREGISTERED_WORKER_GRACE {
        return Ok((true, false));
    }

    tracing::info!(
        event = "agent.bg_run.stale_task_cleared",
        marker = %marker.display(),
    );
    fs::remove_file(marker)?;
    Ok((false, true))
}

pub(crate) fn clear(session_id: &str, task_id: &str) -> Result<()> {
    clear_in(&tasks_dir()?, session_id, task_id)?;
    session_status::touch_session(session_id)
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

/// Adds the runtime marker consumed by shared session background-task logic.
/// Registry failures are treated as pending so automation cannot mistake an
/// unreadable task registry for a completed command.
pub(crate) fn include_pending_status(session: &mut Session) {
    match tasks_dir().and_then(|root| has_pending_in(&root, &session.session_id)) {
        Ok(true) => mark_pending_status(session),
        Ok(false) => {}
        Err(error) => {
            tracing::warn!(
                event = "agent.bg_run.registry_read_failed",
                session = %session.session_id,
                error = %error,
            );
            mark_pending_status(session);
        }
    }
}

pub(crate) fn mark_pending_status(session: &mut Session) {
    session
        .pending_bg_task_ids
        .insert(BG_RUN_PENDING_TASK_MARKER.to_string());
}

pub(crate) fn include_pending_status_in(session: &mut Session, tasks_root: &Path) {
    match has_pending_for_hook_in(tasks_root, &session.session_id) {
        Ok(true) => mark_pending_status(session),
        Ok(false) => {}
        Err(error) => {
            tracing::warn!(
                event = "agent.bg_run.registry_read_failed",
                session = %session.session_id,
                error = %error,
            );
            mark_pending_status(session);
        }
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
