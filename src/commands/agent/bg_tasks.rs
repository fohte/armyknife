use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::{bg_storage, store};
use crate::infra::process;
use crate::shared::cache;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Covers the interval between task registration and the worker recording its PID.
const UNREGISTERED_WORKER_GRACE: Duration = Duration::from_secs(5 * 60);
const REDACTED_CREDENTIAL: &str = "[REDACTED]";

#[derive(Deserialize, Serialize)]
struct TaskRecord {
    parent_pid: Option<u32>,
    worker_pid: Option<u32>,
    command_pid: Option<u32>,
    #[serde(default)]
    stdout_path: Option<PathBuf>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    command: Option<Vec<String>>,
    #[serde(default)]
    started_at: Option<DateTime<Utc>>,
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
    command: &[String],
    stdout_path: &Path,
) -> Result<()> {
    let tasks_root = tasks_dir()?;
    register_in(
        &tasks_root,
        session_id,
        task_id,
        description,
        command,
        stdout_path,
    )?;
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
    command: &[String],
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
                command: Some(command.to_vec()),
                started_at: Some(Utc::now()),
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
    pub command: Option<Vec<String>>,
    pub started_at: DateTime<Utc>,
    pub elapsed_seconds: u64,
    pub stdout_path: PathBuf,
}

pub(crate) fn redact_command_arguments(arguments: &[String]) -> Vec<String> {
    let mut redact_next = false;
    arguments
        .iter()
        .map(|argument| {
            if redact_next && !argument.starts_with('-') {
                redact_next = false;
                return REDACTED_CREDENTIAL.to_owned();
            }
            redact_next = false;

            if let Some((key, _)) = argument.split_once('=')
                && is_sensitive_key(key)
            {
                return format!("{key}={REDACTED_CREDENTIAL}");
            }
            if is_sensitive_header(argument) {
                return REDACTED_CREDENTIAL.to_owned();
            }
            if argument.starts_with('-') && is_sensitive_key(argument) {
                redact_next = true;
                return argument.clone();
            }
            argument.clone()
        })
        .collect()
}

fn is_sensitive_key(key: &str) -> bool {
    let characters = key.trim_start_matches('-').chars().collect::<Vec<_>>();
    let mut words = Vec::new();
    let mut current_word = String::new();

    for (index, character) in characters.iter().copied().enumerate() {
        if !character.is_ascii_alphanumeric() {
            if !current_word.is_empty() {
                words.push(std::mem::take(&mut current_word));
            }
            continue;
        }

        let previous = index
            .checked_sub(1)
            .and_then(|previous_index| characters.get(previous_index))
            .copied();
        let next = characters.get(index + 1).copied();
        let camel_case_boundary = character.is_ascii_uppercase()
            && previous.is_some_and(|previous| {
                previous.is_ascii_lowercase()
                    || previous.is_ascii_digit()
                    || (previous.is_ascii_uppercase()
                        && next.is_some_and(|next| next.is_ascii_lowercase()))
            });
        if camel_case_boundary && !current_word.is_empty() {
            words.push(std::mem::take(&mut current_word));
        }
        current_word.push(character.to_ascii_lowercase());
    }
    if !current_word.is_empty() {
        words.push(current_word);
    }

    if words.iter().any(|word| {
        matches!(
            word.as_str(),
            "token"
                | "password"
                | "passwd"
                | "secret"
                | "credential"
                | "credentials"
                | "authorization"
                | "auth"
        )
    }) {
        return true;
    }

    (words.len() == 1
        && matches!(
            words[0].as_str(),
            "key" | "apikey" | "accesstoken" | "privatekey" | "clientkey" | "signingkey"
        ))
        || words.windows(2).any(|pair| {
            matches!(
                (pair[0].as_str(), pair[1].as_str()),
                ("api", "key")
                    | ("access", "key")
                    | ("private", "key")
                    | ("client", "key")
                    | ("signing", "key")
            )
        })
}

fn is_sensitive_header(argument: &str) -> bool {
    argument
        .split_once(':')
        .is_some_and(|(name, _)| is_sensitive_key(name.trim()))
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
                    let modified = fs::metadata(&path)
                        .ok()
                        .and_then(|metadata| metadata.modified().ok());
                    let elapsed_seconds = modified
                        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                        .unwrap_or_default()
                        .as_secs();
                    let started_at = record
                        .as_ref()
                        .and_then(|record| record.started_at)
                        .or_else(|| modified.map(DateTime::<Utc>::from))
                        .unwrap_or_else(Utc::now);
                    let description = record
                        .as_ref()
                        .and_then(|record| record.description.clone())
                        .unwrap_or_else(|| "Background command".to_owned());
                    let command = record.as_ref().and_then(|record| record.command.clone());
                    let stdout_path = record
                        .and_then(|record| record.stdout_path)
                        .unwrap_or_else(|| super::bg::stdout_path(task_id));
                    tasks.push(PendingTask {
                        id: task_id.to_owned(),
                        description,
                        command,
                        started_at,
                        elapsed_seconds,
                        stdout_path,
                    });
                }
                MarkerState::ClearedStale => stale_cleared = true,
                MarkerState::NotPending => {}
            }
        }
        tasks.sort_by(|left, right| left.id.cmp(&right.id));
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

    use chrono::{DateTime, Utc};
    use rstest::rstest;
    use serde_json::json;
    use tempfile::TempDir;

    use super::{PendingTask, REDACTED_CREDENTIAL, marker_path, pending_tasks_in, register_in};
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
            &["synthetic-command".to_owned(), "--flag".to_owned()],
            &stdout_path,
        )
        .expect("background task should be registered");

        let marker = marker_path(tasks_root.path(), session_id, &task_id)
            .expect("task marker path should be valid");
        let mut record: super::TaskRecord = serde_json::from_slice(
            &std::fs::read(&marker).expect("task marker should be readable"),
        )
        .expect("task marker should deserialize");
        let expected_started_at = DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z")
            .expect("fixture timestamp should parse")
            .with_timezone(&Utc);
        record.started_at = Some(expected_started_at);
        super::write_record(&marker, &record).expect("task marker should update");

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
                    command: Some(vec!["synthetic-command".to_owned(), "--flag".to_owned(),]),
                    started_at: expected_started_at,
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
        let marker_path = session_directory.join(format!("{task_id}.pending"));
        std::fs::write(
            &marker_path,
            serde_json::to_vec(&json!({
                "parent_pid": std::process::id(),
                "worker_pid": null,
                "command_pid": null,
            }))
            .expect("legacy task marker should serialize"),
        )
        .expect("legacy task marker should be written");
        let expected_started_at = chrono::DateTime::<chrono::Utc>::from(
            std::fs::metadata(&marker_path)
                .expect("legacy task marker metadata should be readable")
                .modified()
                .expect("legacy task marker modification time should be readable"),
        );

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
                    command: None,
                    started_at: expected_started_at,
                    elapsed_seconds: 0,
                    stdout_path: super::super::bg::stdout_path(&task_id),
                }],
                false,
            ),
        );
    }

    #[rstest]
    #[case::separate_token_value(
        &["a", "--token", "fixture-secret", "review"],
        vec!["a", "--token", REDACTED_CREDENTIAL, "review"],
    )]
    #[case::separate_password_value(
        &["a", "--password", "fixture-secret"],
        vec!["a", "--password", REDACTED_CREDENTIAL],
    )]
    #[case::camel_case_secret_key(
        &["a", "--clientSecret=fixture-secret"],
        vec!["a", "--clientSecret=[REDACTED]"],
    )]
    #[case::inline_api_key_value(
        &["a", "--api-key=fixture-secret"],
        vec!["a", "--api-key=[REDACTED]"],
    )]
    #[case::environment_api_key_value(
        &["API_KEY=fixture-secret"],
        vec!["API_KEY=[REDACTED]"],
    )]
    #[case::environment_token_value(
        &["GITHUB_TOKEN=fixture-secret"],
        vec!["GITHUB_TOKEN=[REDACTED]"],
    )]
    #[case::auth_subcommand_is_preserved(
        &["a", "gh", "auth", "status"],
        vec!["a", "gh", "auth", "status"],
    )]
    #[case::author_flag_is_preserved(
        &["a", "gh", "pr", "list", "--author", "fixture-author"],
        vec!["a", "gh", "pr", "list", "--author", "fixture-author"],
    )]
    #[case::token_substring_is_not_sensitive(
        &["TOKENIZED=fixture-value"],
        vec!["TOKENIZED=fixture-value"],
    )]
    #[case::authorization_header_value(
        &["a", "-H", "Authorization: Bearer fixture-secret"],
        vec!["a", "-H", REDACTED_CREDENTIAL],
    )]
    fn redacts_credential_arguments(#[case] arguments: &[&str], #[case] expected: Vec<&str>) {
        let arguments = arguments
            .iter()
            .map(|argument| (*argument).to_owned())
            .collect::<Vec<_>>();

        assert_eq!(
            super::redact_command_arguments(&arguments),
            expected.into_iter().map(str::to_owned).collect::<Vec<_>>(),
        );
    }
}
