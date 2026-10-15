use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use uuid::Uuid;

use super::super::bg_storage;
use crate::shared::cache;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct UndeliveredNotification {
    pub task_id: String,
    pub message: String,
}

fn root_dir() -> Result<PathBuf> {
    let base_dir = cache::base_dir().context("Unable to determine cache directory")?;
    Ok(base_dir.join("agent-bg").join("undelivered"))
}

fn session_dir(root: &Path, session_id: &str) -> PathBuf {
    bg_storage::session_dir(root, session_id)
}

fn message_path(root: &Path, session_id: &str, task_id: &str) -> Result<PathBuf> {
    let task_id = bg_storage::task_id(task_id)?;
    Ok(session_dir(root, session_id).join(task_id.to_string()))
}

pub(super) fn save(session_id: &str, task_id: &str, message: &str) -> Result<()> {
    save_in(&root_dir()?, session_id, task_id, message)
}

fn save_in(root: &Path, session_id: &str, task_id: &str, message: &str) -> Result<()> {
    let path = message_path(root, session_id, task_id)?;
    let dir = path
        .parent()
        .context("undelivered notification path has no parent directory")?;
    bg_storage::create_dir_secure(root)?;
    bg_storage::create_dir_secure(dir).with_context(|| {
        format!(
            "failed to create undelivered notification directory: {}",
            dir.display()
        )
    })?;

    bg_storage::write_atomic(&path, |file| {
        file.write_all(message.as_bytes())?;
        Ok(())
    })
}

pub(super) fn try_lock_session(session_id: &str) -> Result<Option<File>> {
    try_lock_session_in(&root_dir()?, session_id)
}

fn try_lock_session_in(root: &Path, session_id: &str) -> Result<Option<File>> {
    let dir = session_dir(root, session_id);
    if !dir.is_dir() {
        return Ok(None);
    }
    let lock_path = dir.join(".redelivery.lock");
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&lock_path)
        .with_context(|| format!("failed to open redelivery lock: {}", lock_path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(error)) => Err(error)
            .with_context(|| format!("failed to lock redelivery session: {}", lock_path.display())),
    }
}

#[cfg(not(test))]
pub(super) fn has_pending(session_id: &str) -> Result<bool> {
    has_pending_in(&root_dir()?, session_id)
}

fn has_pending_in(root: &Path, session_id: &str) -> Result<bool> {
    let dir = session_dir(root, session_id);
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
                .file_name()
                .to_str()
                .is_some_and(|name| Uuid::parse_str(name).is_ok())
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn load_all(session_id: &str) -> Result<Vec<UndeliveredNotification>> {
    load_all_in(&root_dir()?, session_id)
}

fn load_all_in(root: &Path, session_id: &str) -> Result<Vec<UndeliveredNotification>> {
    let dir = session_dir(root, session_id);
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", dir.display()));
        }
    };

    let mut notifications = Vec::new();
    for entry in entries {
        let entry =
            entry.with_context(|| format!("failed to read an entry in {}", dir.display()))?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let Some(task_id) = entry
            .file_name()
            .to_str()
            .and_then(|name| Uuid::parse_str(name).ok())
            .map(|id| id.to_string())
        else {
            continue;
        };
        let path = entry.path();
        let modified = entry
            .metadata()?
            .modified()
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let message = fs::read_to_string(&path).with_context(|| {
            format!(
                "failed to read undelivered notification: {}",
                path.display()
            )
        })?;
        notifications.push((modified, UndeliveredNotification { task_id, message }));
    }
    notifications.sort_by(|(left_modified, left), (right_modified, right)| {
        left_modified
            .cmp(right_modified)
            .then_with(|| left.task_id.cmp(&right.task_id))
    });
    Ok(notifications
        .into_iter()
        .map(|(_, notification)| notification)
        .collect())
}

pub(super) fn remove(session_id: &str, task_id: &str) -> Result<()> {
    remove_in(&root_dir()?, session_id, task_id)
}

fn remove_in(root: &Path, session_id: &str, task_id: &str) -> Result<()> {
    let path = message_path(root, session_id, task_id)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to remove undelivered notification: {}",
                path.display()
            )
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::fixture;
    use tempfile::TempDir;

    const SESSION_ID: &str = "fixture-session";
    const TASK_ID: &str = "a1b2c3d4-e5f6-4789-8abc-def012345678";

    #[fixture]
    fn root() -> (TempDir, PathBuf) {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let root = temp_dir.path().join("agent-bg").join("undelivered");
        (temp_dir, root)
    }

    #[test]
    fn saves_notification_under_session_scoped_task_id() {
        let (_temp_dir, root) = root();
        let message = "<background-task-complete>fixture</background-task-complete>";

        save_in(&root, SESSION_ID, TASK_ID, message).expect("notification should be saved");

        let saved_path = message_path(&root, SESSION_ID, TASK_ID).expect("path should be valid");
        let actual = (
            saved_path.clone(),
            fs::read_to_string(saved_path).expect("notification should be readable"),
            has_pending_in(&root, SESSION_ID).expect("pending scan should succeed"),
            load_all_in(&root, SESSION_ID).expect("notifications should be loadable"),
        );
        assert_eq!(
            actual,
            (
                root.join(crate::shared::hex::encode(SESSION_ID.as_bytes()))
                    .join(TASK_ID),
                message.to_string(),
                true,
                vec![UndeliveredNotification {
                    task_id: TASK_ID.to_string(),
                    message: message.to_string(),
                }],
            ),
        );
    }

    #[test]
    fn loads_notifications_in_save_order() {
        let (_temp_dir, root) = root();
        let earlier_task_id = "a1b2c3d4-e5f6-4789-8abc-def012345678";
        let later_task_id = "b2c3d4e5-f607-489a-bcde-f0123456789a";
        save_in(&root, SESSION_ID, later_task_id, "later message")
            .expect("later notification should be saved");
        save_in(&root, SESSION_ID, earlier_task_id, "earlier message")
            .expect("earlier notification should be saved");
        let directory = session_dir(&root, SESSION_ID);
        File::options()
            .write(true)
            .open(directory.join(later_task_id))
            .expect("later notification should open")
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(2)),
            )
            .expect("later timestamp should be set");
        File::options()
            .write(true)
            .open(directory.join(earlier_task_id))
            .expect("earlier notification should open")
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1)),
            )
            .expect("earlier timestamp should be set");

        let actual = load_all_in(&root, SESSION_ID)
            .expect("notifications should be loadable")
            .into_iter()
            .map(|notification| (notification.task_id, notification.message))
            .collect::<Vec<_>>();
        assert_eq!(
            actual,
            vec![
                (earlier_task_id.to_string(), "earlier message".to_string()),
                (later_task_id.to_string(), "later message".to_string()),
            ],
        );
    }

    #[test]
    fn holds_only_one_redelivery_lock_per_session() {
        let (_temp_dir, root) = root();
        save_in(&root, SESSION_ID, TASK_ID, "fixture message")
            .expect("notification should be saved");

        let first_lock = try_lock_session_in(&root, SESSION_ID)
            .expect("first lock should be acquired")
            .expect("first lock should be available");
        let second_lock = try_lock_session_in(&root, SESSION_ID)
            .expect("second lock attempt should succeed")
            .is_some();
        drop(first_lock);
        let lock_after_release = try_lock_session_in(&root, SESSION_ID)
            .expect("lock after release should succeed")
            .is_some();

        assert_eq!((second_lock, lock_after_release), (false, true));
    }

    #[test]
    fn removes_notification_after_delivery() {
        let (_temp_dir, root) = root();
        save_in(&root, SESSION_ID, TASK_ID, "fixture message")
            .expect("notification should be saved");

        remove_in(&root, SESSION_ID, TASK_ID).expect("notification should be removed");

        assert_eq!(
            (
                load_all_in(&root, SESSION_ID)
                    .expect("pending scan should succeed")
                    .len(),
                has_pending_in(&root, SESSION_ID).expect("pending scan should succeed"),
            ),
            (0, false),
        );
    }
}
