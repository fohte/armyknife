use std::fs;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension,
    backup::{Backup, StepResult},
    params,
};
use serde_json::Value;

use super::super::super::super::history::CodexThreadMetadata;

pub(super) const STATE_DB_FILENAME: &str = "state_5.sqlite";
pub(super) const THREAD_HISTORY_DB_FILENAME: &str = "thread_history_1.sqlite";

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
        let source = Connection::open_with_flags(
            &source_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
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
    use std::time::Duration;

    use rusqlite::{Connection, OpenFlags};
    use tempfile::TempDir;

    use super::{STATE_DB_FILENAME, backup_databases};

    #[test]
    fn online_backup_includes_committed_wal_pages_without_writing_the_source() -> anyhow::Result<()>
    {
        let root = TempDir::new()?;
        let root_path = root.path().canonicalize()?;
        let source_home = root_path.join("source");
        let private_home = root_path.join("private");
        std::fs::create_dir_all(&source_home)?;
        std::fs::create_dir_all(&private_home)?;
        let source_path = source_home.join(STATE_DB_FILENAME);
        let source = Connection::open(&source_path)?;
        source.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT NOT NULL, history_mode TEXT NOT NULL);
             INSERT INTO threads VALUES ('fixture-thread', '/source/sessions/fixture.jsonl', 'paginated');",
        )?;
        source.busy_timeout(Duration::from_secs(1))?;
        Connection::open(source_home.join(super::THREAD_HISTORY_DB_FILENAME))?
            .execute_batch("CREATE TABLE fixture_history (id TEXT)")?;

        backup_databases(
            &source_home,
            &private_home,
            std::time::Instant::now() + Duration::from_secs(10),
        )?;
        let copied = Connection::open_with_flags(
            private_home.join(STATE_DB_FILENAME),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let copied_row: (String, String) = copied.query_row(
            "SELECT rollout_path, history_mode FROM threads WHERE id = ?1",
            ["fixture-thread"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let original_row: (String, String) = source.query_row(
            "SELECT rollout_path, history_mode FROM threads WHERE id = ?1",
            ["fixture-thread"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        assert_eq!(
            (
                copied_row,
                original_row,
                source_path.exists(),
                source_home
                    .join(format!("{STATE_DB_FILENAME}-wal"))
                    .exists(),
            ),
            (
                (
                    "/source/sessions/fixture.jsonl".to_string(),
                    "paginated".to_string(),
                ),
                (
                    "/source/sessions/fixture.jsonl".to_string(),
                    "paginated".to_string(),
                ),
                true,
                true,
            ),
        );
        Ok(())
    }
}
