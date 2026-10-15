use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use uuid::Uuid;

use crate::shared::{cache, hex};

pub(super) struct UndeliveredNotification {
    pub task_id: String,
    pub message: String,
}

fn root_dir() -> Result<PathBuf> {
    let base_dir = cache::base_dir().context("Unable to determine cache directory")?;
    Ok(base_dir.join("agent-bg").join("undelivered"))
}

fn session_dir(root: &Path, session_id: &str) -> PathBuf {
    root.join(hex::encode(session_id.as_bytes()))
}

fn message_path(root: &Path, session_id: &str, task_id: &str) -> Result<PathBuf> {
    let task_id = Uuid::parse_str(task_id).context("invalid background task ID")?;
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
    create_dir_secure(root)?;
    create_dir_secure(dir).with_context(|| {
        format!(
            "failed to create undelivered notification directory: {}",
            dir.display()
        )
    })?;

    let temp_path = dir.join(format!(".{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp_path)
            .with_context(|| {
                format!(
                    "failed to create temporary undelivered notification: {}",
                    temp_path.display()
                )
            })?;
        file.write_all(message.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp_path, &path).with_context(|| {
            format!(
                "failed to save undelivered notification: {}",
                path.display()
            )
        })?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
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
        let message = fs::read_to_string(&path).with_context(|| {
            format!(
                "failed to read undelivered notification: {}",
                path.display()
            )
        })?;
        notifications.push(UndeliveredNotification { task_id, message });
    }
    notifications.sort_by(|left, right| left.task_id.cmp(&right.task_id));
    Ok(notifications)
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

fn create_dir_secure(dir: &Path) -> std::io::Result<()> {
    if !dir.exists() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }

    let metadata = fs::metadata(dir)?;
    let mut permissions = metadata.permissions();
    if permissions.mode() & 0o077 != 0 {
        permissions.set_mode(0o700);
        fs::set_permissions(dir, permissions)?;
    }
    Ok(())
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
        );
        assert_eq!(
            actual,
            (
                root.join(hex::encode(SESSION_ID.as_bytes())).join(TASK_ID),
                message.to_string(),
                true,
            ),
        );
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
