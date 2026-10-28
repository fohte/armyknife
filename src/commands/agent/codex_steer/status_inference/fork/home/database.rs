use std::fs;
use std::io::Read;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension,
    backup::{Backup, StepResult},
    params,
};
use serde_json::Value;

use super::super::super::super::history::CodexThreadMetadata;

pub(super) const STATE_DB_FILENAME: &str = "state_5.sqlite";
pub(super) const THREAD_HISTORY_DB_FILENAME: &str = "thread_history_1.sqlite";

const SQLITE_URI_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'&')
    .add(b'?');

pub(super) struct ThreadRecord {
    pub(super) rollout_path: String,
    pub(super) history_mode: String,
}

pub(super) fn backup_databases(
    source_home: &Path,
    private_home: &Path,
    deadline: Instant,
) -> Result<()> {
    for filename in [STATE_DB_FILENAME, THREAD_HISTORY_DB_FILENAME] {
        let source_path = source_home.join(filename);
        let destination_path = private_home.join(filename);
        let metadata = fs::symlink_metadata(&source_path)
            .with_context(|| format!("inspect source Codex {filename}"))?;
        ensure!(
            metadata.file_type().is_file(),
            "source Codex database is not a regular file"
        );
        ensure_wal_sidecars_are_readable(&source_path)?;
        let source_uri = readonly_source_uri(&source_path)?;
        let source = Connection::open_with_flags(
            source_uri,
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW
                | OpenFlags::SQLITE_OPEN_URI,
        )
        .with_context(|| format!("open source Codex {filename} read-only"))?;
        source
            .busy_timeout(Duration::from_secs(10))
            .with_context(|| format!("set source Codex {filename} backup timeout"))?;
        let mut destination = Connection::open(&destination_path)
            .with_context(|| format!("create private Codex {filename}"))?;
        {
            let backup = Backup::new(&source, &mut destination)
                .with_context(|| format!("start online backup of Codex {filename}"))?;
            loop {
                ensure!(
                    Instant::now() < deadline,
                    "Codex private home snapshot exceeded its time limit"
                );
                match backup
                    .step(128)
                    .with_context(|| format!("complete online backup of Codex {filename}"))?
                {
                    StepResult::Done => break,
                    StepResult::More | StepResult::Busy | StepResult::Locked => {
                        thread::sleep(Duration::from_millis(25));
                    }
                    _ => bail!("Codex SQLite online backup returned an unsupported state"),
                }
            }
        }
        destination
            .close()
            .map_err(|(_, error)| error)
            .with_context(|| format!("close private Codex {filename} backup"))?;
    }
    Ok(())
}

fn readonly_source_uri(path: &Path) -> Result<String> {
    let path = path
        .to_str()
        .context("source Codex database path is not valid UTF-8")?;
    // SQLite's bundled Unix VFS otherwise opens the WAL index with write access.
    Ok(format!(
        "file:{}?mode=ro&readonly_shm=1",
        utf8_percent_encode(path, SQLITE_URI_ENCODE_SET)
    ))
}

fn ensure_wal_sidecars_are_readable(database_path: &Path) -> Result<()> {
    let mut database =
        fs::File::open(database_path).context("read source Codex database header")?;
    let mut header = [0; 20];
    database
        .read_exact(&mut header)
        .context("read source Codex database header")?;
    if header[18] != 2 || header[19] != 2 {
        return Ok(());
    }

    for suffix in ["-wal", "-shm"] {
        let sidecar = database_path.with_file_name(format!(
            "{}{}",
            database_path
                .file_name()
                .context("source Codex database has no filename")?
                .to_string_lossy(),
            suffix
        ));
        let metadata = fs::symlink_metadata(&sidecar)
            .with_context(|| format!("inspect source Codex WAL sidecar {suffix}"))?;
        ensure!(
            metadata.file_type().is_file(),
            "source Codex WAL sidecar is not a regular file"
        );
        fs::File::open(sidecar)
            .with_context(|| format!("open source Codex WAL sidecar {suffix} read-only"))?;
    }
    Ok(())
}

pub(super) fn read_thread_record(connection: &Connection, thread_id: &str) -> Result<ThreadRecord> {
    let record = connection
        .query_row(
            "SELECT rollout_path, history_mode FROM threads WHERE id = ?1",
            [thread_id],
            |row| {
                Ok(ThreadRecord {
                    rollout_path: row.get(0)?,
                    history_mode: row.get(1)?,
                })
            },
        )
        .optional()
        .context("read Codex thread record from private state backup")?;
    record.with_context(|| "Codex thread is missing from private state backup".to_string())
}

pub(super) fn rewrite_rollout_path(
    connection: &Connection,
    thread_id: &str,
    rollout_path: &Path,
) -> Result<()> {
    let rollout_path = rollout_path
        .to_str()
        .context("private Codex rollout path is not valid UTF-8")?;
    let changed = connection
        .execute(
            "UPDATE threads SET rollout_path = ?1 WHERE id = ?2",
            params![rollout_path, thread_id],
        )
        .context("rewire private Codex rollout path")?;
    ensure!(
        changed == 1,
        "private Codex rollout reference is inconsistent"
    );
    Ok(())
}

pub(super) fn validate_target_history(path: &Path, thread: &CodexThreadMetadata) -> Result<()> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .context("open private Codex thread-history backup")?;
    let turn_status = connection
        .query_row(
            "SELECT status FROM thread_turns WHERE thread_id = ?1 AND turn_id = ?2",
            params![thread.session_id, thread.turn_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .context("read target turn from private Codex history backup")?
        .context("target turn is missing from private Codex history backup")?;
    ensure!(
        matches!(turn_status.as_str(), "completed" | "interrupted" | "failed"),
        "target turn is not terminal in private Codex history backup"
    );

    let mut statement = connection
        .prepare(
            "SELECT item_json FROM thread_items
             WHERE thread_id = ?1 AND turn_id = ?2
             ORDER BY rollout_ordinal DESC",
        )
        .context("prepare target items lookup in private Codex history backup")?;
    let mut rows = statement
        .query(params![thread.session_id, thread.turn_id])
        .context("read target items from private Codex history backup")?;
    let mut final_assistant_message = None;
    while let Some(row) = rows
        .next()
        .context("iterate target items in private Codex history backup")?
    {
        let item_json = row
            .get::<_, String>(0)
            .context("read target item from private Codex history backup")?;
        let item: Value = serde_json::from_str(&item_json)
            .context("parse target item in private Codex history backup")?;
        if item.get("type").and_then(Value::as_str) == Some("agentMessage") {
            final_assistant_message = item
                .get("text")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            break;
        }
    }
    ensure!(
        final_assistant_message.as_deref() == Some(thread.last_assistant_message.as_str()),
        "target final assistant message does not match private Codex history backup"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::time::Duration;
    use std::time::SystemTime;

    use rusqlite::{Connection, OpenFlags};
    use tempfile::TempDir;

    use super::{STATE_DB_FILENAME, THREAD_HISTORY_DB_FILENAME, backup_databases};

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct FileObservation {
        contents: Vec<u8>,
        modified: Option<SystemTime>,
    }

    fn snapshot_directory(
        path: &std::path::Path,
    ) -> anyhow::Result<BTreeMap<OsString, FileObservation>> {
        std::fs::read_dir(path)?
            .map(|entry| {
                let entry = entry?;
                let metadata = entry.metadata()?;
                Ok((
                    entry.file_name(),
                    FileObservation {
                        contents: std::fs::read(entry.path())?,
                        modified: metadata.modified().ok(),
                    },
                ))
            })
            .collect()
    }

    #[test]
    fn online_backup_includes_committed_wal_pages_without_writing_the_source() -> anyhow::Result<()>
    {
        let root = TempDir::new()?;
        let root_path = root.path().canonicalize()?;
        let writer_home = root_path.join("writer");
        let source_home = root_path.join("source");
        let private_home = root_path.join("private");
        std::fs::create_dir_all(&writer_home)?;
        std::fs::create_dir_all(&source_home)?;
        std::fs::create_dir_all(&private_home)?;
        let source_path = source_home.join(STATE_DB_FILENAME);
        let writer_path = writer_home.join(STATE_DB_FILENAME);
        let writer = Connection::open(&writer_path)?;
        writer.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT NOT NULL, history_mode TEXT NOT NULL);
             INSERT INTO threads VALUES ('fixture-thread', '/source/sessions/fixture.jsonl', 'paginated');",
        )?;
        writer.busy_timeout(Duration::from_secs(1))?;
        Connection::open(writer_home.join(THREAD_HISTORY_DB_FILENAME))?
            .execute_batch("CREATE TABLE fixture_history (id TEXT)")?;
        for suffix in ["", "-wal", "-shm"] {
            std::fs::copy(
                writer_path.with_file_name(format!("{STATE_DB_FILENAME}{suffix}")),
                source_home.join(format!("{STATE_DB_FILENAME}{suffix}")),
            )?;
        }
        std::fs::copy(
            writer_home.join(THREAD_HISTORY_DB_FILENAME),
            source_home.join(THREAD_HISTORY_DB_FILENAME),
        )?;
        drop(writer);
        let source_before = snapshot_directory(&source_home)?;

        backup_databases(
            &source_home,
            &private_home,
            std::time::Instant::now() + Duration::from_secs(10),
        )?;
        let source_after = snapshot_directory(&source_home)?;
        let copied = Connection::open_with_flags(
            private_home.join(STATE_DB_FILENAME),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let copied_row: (String, String) = copied.query_row(
            "SELECT rollout_path, history_mode FROM threads WHERE id = ?1",
            ["fixture-thread"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(
            (
                copied_row,
                source_path.exists(),
                source_home
                    .join(format!("{STATE_DB_FILENAME}-wal"))
                    .exists(),
                source_before == source_after,
            ),
            (
                (
                    "/source/sessions/fixture.jsonl".to_string(),
                    "paginated".to_string(),
                ),
                true,
                true,
                true,
            ),
        );
        Ok(())
    }

    #[test]
    fn missing_wal_sidecar_fails_before_opening_or_changing_the_source() -> anyhow::Result<()> {
        let root = TempDir::new()?;
        let root_path = root.path().canonicalize()?;
        let writer_home = root_path.join("writer");
        let source_home = root_path.join("missing-sidecar-source");
        let private_home = root_path.join("private");
        std::fs::create_dir_all(&writer_home)?;
        std::fs::create_dir_all(&source_home)?;
        std::fs::create_dir_all(&private_home)?;
        let source_path = writer_home.join(STATE_DB_FILENAME);
        let writer = Connection::open(&source_path)?;
        writer.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA wal_autocheckpoint=0;
             CREATE TABLE fixture (id TEXT);
             INSERT INTO fixture VALUES ('fixture-row');",
        )?;
        let wal_path = source_path.with_file_name(format!("{STATE_DB_FILENAME}-wal"));
        std::fs::copy(&source_path, source_home.join(STATE_DB_FILENAME))?;
        std::fs::copy(
            &wal_path,
            source_home.join(format!("{STATE_DB_FILENAME}-wal")),
        )?;
        drop(writer);
        let before = snapshot_directory(&source_home)?;

        let result = backup_databases(
            &source_home,
            &private_home,
            std::time::Instant::now() + Duration::from_secs(10),
        )
        .map_err(|_| ());

        assert_eq!(
            (result, snapshot_directory(&source_home)?, before.clone()),
            (Err(()), before.clone(), before),
        );
        Ok(())
    }
}
