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
    params, params_from_iter,
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

pub(super) fn backup_state_database(
    source_path: &Path,
    destination_path: &Path,
    deadline: Instant,
) -> Result<()> {
    validate_database_file(source_path)?;
    ensure_wal_sidecars_are_readable(source_path)?;
    let source_uri = readonly_source_uri(source_path)?;
    let source = Connection::open_with_flags(
        source_uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW
            | OpenFlags::SQLITE_OPEN_URI,
    )
    .context("open source Codex state database read-only")?;
    source
        .busy_timeout(Duration::from_secs(10))
        .context("set source Codex state backup timeout")?;
    let mut destination =
        Connection::open(destination_path).context("create private Codex state database")?;
    backup_in_one_step(&source, &mut destination, deadline)?;
    destination
        .close()
        .map_err(|(_, error)| error)
        .context("close private Codex state database backup")?;
    Ok(())
}

fn backup_in_one_step(
    source: &Connection,
    destination: &mut Connection,
    deadline: Instant,
) -> Result<()> {
    {
        let backup = Backup::new(source, destination)
            .context("start online backup of Codex state database")?;
        loop {
            ensure!(
                Instant::now() < deadline,
                "Codex private home snapshot exceeded its time limit"
            );
            match backup
                .step(-1)
                .context("complete online backup of Codex state database")?
            {
                StepResult::Done => break,
                StepResult::Busy | StepResult::Locked => {
                    thread::sleep(Duration::from_millis(25));
                }
                StepResult::More => {
                    bail!("Codex SQLite full backup did not complete in one step")
                }
                _ => bail!("Codex SQLite online backup returned an unsupported state"),
            }
        }
    }
    Ok(())
}

pub(super) fn copy_thread_history_database(
    source_path: &Path,
    destination_path: &Path,
    lineage_thread_ids: &[String],
    deadline: Instant,
) -> Result<()> {
    validate_database_file(source_path)?;
    ensure_wal_sidecars_are_readable(source_path)?;
    ensure!(
        !lineage_thread_ids.is_empty(),
        "Codex thread history lineage is empty"
    );
    let source_uri = readonly_source_uri(source_path)?;
    let destination = Connection::open(destination_path)
        .context("create private Codex thread-history database")?;
    destination
        .busy_timeout(Duration::from_secs(10))
        .context("set private Codex thread-history timeout")?;
    destination
        .execute("ATTACH DATABASE ?1 AS source", [&source_uri])
        .context("attach source Codex thread history read-only")?;
    destination
        .execute_batch("BEGIN IMMEDIATE")
        .context("begin private Codex thread-history copy")?;

    let result = (|| {
        let schema = read_schema(&destination, deadline)?;
        let table_names = schema
            .iter()
            .filter(|object| object.kind == "table")
            .map(|object| object.name.clone())
            .collect::<Vec<_>>();
        for object in schema.iter().filter(|object| object.kind == "table") {
            ensure_snapshot_time(deadline)?;
            destination
                .execute_batch(
                    object
                        .sql
                        .as_deref()
                        .context("Codex history table has no schema")?,
                )
                .with_context(|| format!("create private Codex history table {}", object.name))?;
        }

        for table_name in &table_names {
            ensure_snapshot_time(deadline)?;
            if table_name == "sqlite_sequence" {
                continue;
            }
            let has_thread_id = table_has_column(&destination, table_name, "thread_id")?;
            ensure!(
                has_thread_id || table_name == "_sqlx_migrations",
                "Codex thread-history table has no thread_id column"
            );
            let table = quote_identifier(table_name);
            if has_thread_id {
                let placeholders = (1..=lineage_thread_ids.len())
                    .map(|index| format!("?{index}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let sql = format!(
                    "INSERT INTO main.{table} SELECT * FROM source.{table} WHERE thread_id IN ({placeholders})"
                );
                let ids = lineage_thread_ids
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>();
                destination.execute(&sql, params_from_iter(ids))
            } else {
                let sql = format!("INSERT INTO main.{table} SELECT * FROM source.{table}");
                destination.execute(&sql, [])
            }
            .with_context(|| format!("copy private Codex history table {table_name}"))?;
        }

        let has_sequence_table: bool = destination.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM source.sqlite_master WHERE name = 'sqlite_sequence'
             )",
            [],
            |row| row.get(0),
        )?;
        if has_sequence_table {
            destination
                .execute(
                    "INSERT INTO main.sqlite_sequence SELECT * FROM source.sqlite_sequence",
                    [],
                )
                .context("copy Codex thread-history sequence values")?;
        }

        for object in schema.iter().filter(|object| object.kind != "table") {
            ensure_snapshot_time(deadline)?;
            destination
                .execute_batch(
                    object
                        .sql
                        .as_deref()
                        .context("Codex history object has no schema")?,
                )
                .with_context(|| {
                    format!(
                        "create private Codex history {} {}",
                        object.kind, object.name
                    )
                })?;
        }

        let application_id: i64 =
            destination.pragma_query_value(Some("source"), "application_id", |row| row.get(0))?;
        let user_version: i64 =
            destination.pragma_query_value(Some("source"), "user_version", |row| row.get(0))?;
        destination.pragma_update(None, "application_id", application_id)?;
        destination.pragma_update(None, "user_version", user_version)?;
        ensure_snapshot_time(deadline)?;
        Ok(())
    })();

    match result {
        Ok(()) => destination
            .execute_batch("COMMIT")
            .context("commit private Codex thread-history copy")?,
        Err(error) => {
            let _ = destination.execute_batch("ROLLBACK");
            return Err(error);
        }
    }
    destination
        .execute_batch("DETACH DATABASE source")
        .context("detach source Codex thread history")?;
    destination
        .close()
        .map_err(|(_, error)| error)
        .context("close private Codex thread-history database")?;
    Ok(())
}

fn validate_database_file(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).context("inspect source Codex database")?;
    ensure!(
        metadata.file_type().is_file(),
        "source Codex database is not a regular file"
    );
    Ok(())
}

struct SchemaObject {
    kind: String,
    name: String,
    sql: Option<String>,
}

fn read_schema(connection: &Connection, deadline: Instant) -> Result<Vec<SchemaObject>> {
    ensure_snapshot_time(deadline)?;
    let mut statement = connection
        .prepare(
            "SELECT type, name, sql FROM source.sqlite_master
             WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%'
             ORDER BY CASE type WHEN 'table' THEN 0 ELSE 1 END, name",
        )
        .context("read source Codex thread-history schema")?;
    statement
        .query_map([], |row| {
            Ok(SchemaObject {
                kind: row.get(0)?,
                name: row.get(1)?,
                sql: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("collect source Codex thread-history schema")
}

fn table_has_column(connection: &Connection, table_name: &str, column_name: &str) -> Result<bool> {
    let sql = format!("PRAGMA source.table_info({})", quote_identifier(table_name));
    let mut statement = connection
        .prepare(&sql)
        .context("inspect Codex thread-history table columns")?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(columns.iter().any(|column| column == column_name))
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn ensure_snapshot_time(deadline: Instant) -> Result<()> {
    ensure!(
        Instant::now() < deadline,
        "Codex private home snapshot exceeded its time limit"
    );
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
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::Duration;
    use std::time::SystemTime;

    use rusqlite::{Connection, OpenFlags};
    use tempfile::TempDir;

    use super::{STATE_DB_FILENAME, backup_in_one_step, backup_state_database};

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
        let source_home = root_path.join("source home #1?%&\"");
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
        for suffix in ["", "-wal", "-shm"] {
            std::fs::copy(
                writer_path.with_file_name(format!("{STATE_DB_FILENAME}{suffix}")),
                source_home.join(format!("{STATE_DB_FILENAME}{suffix}")),
            )?;
        }
        drop(writer);
        let source_before = snapshot_directory(&source_home)?;

        backup_state_database(
            &source_path,
            &private_home.join(STATE_DB_FILENAME),
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
    fn full_state_backup_completes_while_the_source_is_being_written() -> anyhow::Result<()> {
        let root = TempDir::new()?;
        let source_home = root.path().join("source");
        let private_home = root.path().join("private");
        std::fs::create_dir_all(&source_home)?;
        std::fs::create_dir_all(&private_home)?;
        let source_path = source_home.join(STATE_DB_FILENAME);
        let source = Connection::open(&source_path)?;
        source.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA wal_autocheckpoint=0;
             CREATE TABLE state_rows (id INTEGER PRIMARY KEY, payload BLOB NOT NULL);
             CREATE TABLE writer_heartbeat (value INTEGER NOT NULL);
             INSERT INTO writer_heartbeat VALUES (0);",
        )?;
        let transaction = source.unchecked_transaction()?;
        for id in 0..8_192 {
            transaction.execute(
                "INSERT INTO state_rows (id, payload) VALUES (?1, zeroblob(4096))",
                [id],
            )?;
        }
        transaction.commit()?;

        let snapshot_source = Connection::open(&source_path)?;

        let is_writing = Arc::new(AtomicBool::new(true));
        let writer_started = Arc::new(AtomicBool::new(false));
        let writer_is_writing = Arc::clone(&is_writing);
        let writer_started_signal = Arc::clone(&writer_started);
        let writer = thread::spawn(move || -> anyhow::Result<Connection> {
            while writer_is_writing.load(Ordering::Relaxed) {
                source.execute("UPDATE writer_heartbeat SET value = value + 1", [])?;
                writer_started_signal.store(true, Ordering::Relaxed);
                thread::sleep(Duration::from_millis(1));
            }
            Ok(source)
        });
        while !writer_started.load(Ordering::Relaxed) {
            thread::yield_now();
        }

        let mut destination = Connection::open(private_home.join(STATE_DB_FILENAME))?;
        let snapshot = backup_in_one_step(
            &snapshot_source,
            &mut destination,
            std::time::Instant::now() + Duration::from_secs(5),
        );
        is_writing.store(false, Ordering::Relaxed);
        drop(writer.join().expect("state writer thread should finish")?);
        snapshot?;
        destination.close().map_err(|(_, error)| error)?;

        let copied = Connection::open_with_flags(
            private_home.join(STATE_DB_FILENAME),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let row_count: i64 =
            copied.query_row("SELECT count(*) FROM state_rows", [], |row| row.get(0))?;
        assert_eq!(row_count, 8_192);
        Ok(())
    }

    #[rstest::rstest]
    #[case::missing_wal("-wal")]
    #[case::missing_shm("-shm")]
    fn missing_wal_sidecar_fails_before_opening_or_changing_the_source(
        #[case] missing_suffix: &str,
    ) -> anyhow::Result<()> {
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
        for suffix in ["", "-wal", "-shm"] {
            if suffix != missing_suffix {
                std::fs::copy(
                    source_path.with_file_name(format!("{STATE_DB_FILENAME}{suffix}")),
                    source_home.join(format!("{STATE_DB_FILENAME}{suffix}")),
                )?;
            }
        }
        drop(writer);
        let before = snapshot_directory(&source_home)?;

        let result = backup_state_database(
            &source_home.join(STATE_DB_FILENAME),
            &private_home.join(STATE_DB_FILENAME),
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
