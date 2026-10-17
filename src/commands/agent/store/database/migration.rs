use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::{DateTime, TimeDelta, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

use crate::commands::agent::types::Session;

const LEGACY_RETENTION: TimeDelta = TimeDelta::days(30);
const LEGACY_LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const LEGACY_LOCK_RETRY: Duration = Duration::from_millis(25);

pub(super) fn import_legacy_json(connection: &mut Connection, sessions_dir: &Path) -> Result<()> {
    let mut paths = legacy_json_paths(sessions_dir)?;
    if paths.is_empty() {
        return Ok(());
    }

    let legacy_locks = loop {
        let locks = lock_legacy_files(&paths)?;
        let current_paths = legacy_json_paths(sessions_dir)?;
        if current_paths != paths {
            drop(locks);
            paths = current_paths;
            if paths.is_empty() {
                return Ok(());
            }
            continue;
        }
        break locks;
    };

    let importable_sessions = decode_legacy_sessions(&paths)?;
    if importable_sessions.is_empty() {
        remove_locked_legacy_lock_files(&paths);
        drop(legacy_locks);
        return Ok(());
    }

    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut valid_paths = Vec::new();

    for (path, session) in importable_sessions {
        let current = super::read_session(&transaction, &session.session_id)?;
        let should_import = current
            .as_ref()
            .is_none_or(|current| session.updated_at > current.updated_at);
        if should_import {
            super::write_session(&transaction, &session)?;
        }
        valid_paths.push(path);
    }

    if !valid_paths.is_empty() {
        transaction.execute(
            "INSERT INTO session_store_meta (key, value) VALUES ('legacy-imported-at', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [Utc::now().to_rfc3339()],
        )?;
    }
    transaction.commit()?;

    move_legacy_json(sessions_dir, &valid_paths);
    remove_locked_legacy_lock_files(&paths);
    drop(legacy_locks);
    Ok(())
}

pub(super) fn remove_legacy_lock_files(sessions_dir: &Path) {
    let Ok(entries) = fs::read_dir(sessions_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_lock_file = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".json.lock"));
        if !is_lock_file {
            continue;
        }

        let Some(lock_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(json_name) = lock_name.strip_suffix(".lock") else {
            continue;
        };
        if path.with_file_name(json_name).exists() {
            continue;
        }
        let lock_file = match OpenOptions::new().read(true).write(true).open(&path) {
            Ok(lock_file) => lock_file,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => {
                warn_legacy_lock_delete_failure(&path, &error);
                continue;
            }
        };
        match lock_file.try_lock() {
            Ok(()) => remove_legacy_lock_file(&path),
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(std::fs::TryLockError::Error(error)) => {
                warn_legacy_lock_delete_failure(&path, &error);
            }
        }
    }
}

fn remove_locked_legacy_lock_files(session_paths: &[PathBuf]) {
    for session_path in session_paths {
        remove_legacy_lock_file(&session_path.with_extension("json.lock"));
    }
}

fn remove_legacy_lock_file(path: &Path) {
    if let Err(error) = fs::remove_file(path)
        && error.kind() != ErrorKind::NotFound
    {
        warn_legacy_lock_delete_failure(path, &error);
    }
}

fn warn_legacy_lock_delete_failure(path: &Path, error: &impl std::fmt::Display) {
    tracing::warn!(
        target: "armyknife::commands::agent::store",
        event = "agent.store.legacy_lock_delete_failed",
        path = %path.display(),
        error = %error,
    );
}

pub(super) fn cleanup_expired_legacy_json(
    connection: &Connection,
    sessions_dir: &Path,
) -> Result<()> {
    let imported_at: Option<String> = connection
        .query_row(
            "SELECT value FROM session_store_meta WHERE key = 'legacy-imported-at'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let Some(imported_at) = imported_at else {
        return Ok(());
    };
    let Ok(imported_at) = DateTime::parse_from_rfc3339(&imported_at) else {
        tracing::warn!(
            target: "armyknife::commands::agent::store",
            event = "agent.store.legacy_import_timestamp_invalid",
        );
        return Ok(());
    };
    if Utc::now() - imported_at.with_timezone(&Utc) < LEGACY_RETENTION {
        return Ok(());
    }

    let legacy_dir = sessions_dir.join("legacy-json");
    if let Err(error) = fs::remove_dir_all(&legacy_dir)
        && error.kind() != ErrorKind::NotFound
    {
        tracing::warn!(
            target: "armyknife::commands::agent::store",
            event = "agent.store.legacy_directory_delete_failed",
            path = %legacy_dir.display(),
            error = %error,
        );
    }
    Ok(())
}

fn legacy_json_paths(sessions_dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = fs::read_dir(sessions_dir)
        .with_context(|| format!("reading sessions dir {}", sessions_dir.display()))?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_file() && path.extension().is_some_and(|ext| ext == "json") {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn decode_legacy_sessions(paths: &[PathBuf]) -> Result<Vec<(PathBuf, Session)>> {
    let mut sessions = Vec::new();
    for path in paths {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("reading legacy session {}", path.display()));
            }
        };
        match serde_json::from_slice::<Session>(&bytes) {
            Ok(session) => match super::validate_session_id(&session.session_id) {
                Ok(()) => sessions.push((path.clone(), session)),
                Err(error) => tracing::warn!(
                    target: "armyknife::commands::agent::store",
                    event = "agent.store.legacy_session_id_invalid",
                    path = %path.display(),
                    error = %error,
                ),
            },
            Err(error) => tracing::warn!(
                target: "armyknife::commands::agent::store",
                event = "agent.store.legacy_session_decode_failed",
                path = %path.display(),
                error = %error,
            ),
        }
    }
    Ok(sessions)
}

fn lock_legacy_files(paths: &[PathBuf]) -> Result<Vec<File>> {
    let mut locks = Vec::with_capacity(paths.len());
    for path in paths {
        let started_at = Instant::now();
        let lock_path = path.with_extension("json.lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| format!("opening legacy session lock {}", lock_path.display()))?;
        loop {
            match file.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock)
                    if started_at.elapsed() < LEGACY_LOCK_TIMEOUT =>
                {
                    std::thread::sleep(LEGACY_LOCK_RETRY);
                }
                Err(std::fs::TryLockError::WouldBlock) => {
                    anyhow::bail!(
                        "timed out waiting for legacy session lock {}",
                        lock_path.display()
                    );
                }
                Err(std::fs::TryLockError::Error(error)) => {
                    return Err(error).with_context(|| {
                        format!("locking legacy session {}", lock_path.display())
                    });
                }
            }
        }
        locks.push(file);
    }
    Ok(locks)
}

fn move_legacy_json(sessions_dir: &Path, paths: &[PathBuf]) {
    if paths.is_empty() {
        return;
    }
    let legacy_dir = sessions_dir.join("legacy-json");
    if let Err(error) = fs::create_dir_all(&legacy_dir) {
        tracing::warn!(
            target: "armyknife::commands::agent::store",
            event = "agent.store.legacy_directory_create_failed",
            path = %legacy_dir.display(),
            error = %error,
        );
        return;
    }
    for path in paths {
        let Some(file_name) = path.file_name() else {
            continue;
        };
        let destination = legacy_dir.join(file_name);
        if let Err(error) = fs::rename(path, &destination)
            && error.kind() != ErrorKind::NotFound
        {
            tracing::warn!(
                target: "armyknife::commands::agent::store",
                event = "agent.store.legacy_session_move_failed",
                path = %path.display(),
                destination = %destination.display(),
                error = %error,
            );
        }
    }
}
