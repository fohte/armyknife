use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::super::super::history::CodexThreadMetadata;
use crate::commands::agent::types::Session;

mod config;
mod database;
mod history;

pub(super) struct PrivateCodexHome {
    pub(super) codex_home: PathBuf,
    pub(super) sqlite_home: PathBuf,
    pub(super) log_dir: PathBuf,
    pub(super) temp_dir: PathBuf,
}

pub(super) fn prepare_private_codex_home(
    inference_dir: &Path,
    session: &Session,
    thread: &CodexThreadMetadata,
) -> Result<PrivateCodexHome> {
    let source = config::resolve_source(&session.cwd)?;
    prepare_from_source(inference_dir, &source, thread)
}

fn prepare_from_source(
    inference_dir: &Path,
    source: &config::SourcePaths,
    thread: &CodexThreadMetadata,
) -> Result<PrivateCodexHome> {
    let codex_home = inference_dir.join("codex-home");
    let sqlite_home = codex_home.join("state");
    let log_dir = codex_home.join("logs");
    let temp_dir = codex_home.join("tmp");
    std::fs::create_dir_all(&sqlite_home).context("create private Codex SQLite home")?;
    std::fs::create_dir_all(&log_dir).context("create private Codex log directory")?;
    std::fs::create_dir_all(&temp_dir).context("create private Codex temporary directory")?;

    config::copy_startup_files(&source.codex_home, &codex_home)?;
    database::backup_databases(&source.sqlite_home, &sqlite_home)?;
    history::copy_lineage_and_rewrite(
        &source.codex_home,
        &codex_home,
        &sqlite_home.join(database::STATE_DB_FILENAME),
        thread,
    )?;
    database::validate_target_history(
        &sqlite_home.join(database::THREAD_HISTORY_DB_FILENAME),
        thread,
    )?;

    Ok(PrivateCodexHome {
        codex_home,
        sqlite_home,
        log_dir,
        temp_dir,
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::path::PathBuf;

    use anyhow::Result;
    use rstest::rstest;
    use rusqlite::Connection;
    use serde_json::json;
    use tempfile::TempDir;

    use super::{config::SourcePaths, prepare_from_source};
    use crate::commands::agent::codex_steer::history::CodexThreadMetadata;

    const THREAD_ID: &str = "fixture-thread";
    const PARENT_ID: &str = "fixture-parent";
    const TURN_ID: &str = "fixture-turn";
    const FINAL_MESSAGE: &str = "The fixture task is waiting.";

    struct Fixture {
        _root: TempDir,
        source: SourcePaths,
        _state: Connection,
        _thread_history: Connection,
        target_rollout: PathBuf,
        parent_rollout: PathBuf,
    }

    fn fixture() -> Result<Fixture> {
        let root = tempfile::Builder::new()
            .prefix("armyknife-codex-home-fixture-")
            .tempdir_in(std::env::temp_dir())?;
        let root_path = root.path().canonicalize()?;
        let codex_home = root_path.join("source-codex-home");
        let sqlite_home = root_path.join("source-sqlite");
        std::fs::create_dir_all(&codex_home)?;
        std::fs::create_dir_all(&sqlite_home)?;
        std::fs::create_dir_all(codex_home.join("sessions/2026/01/02"))?;
        std::fs::write(codex_home.join("config.toml"), "mcp_servers = {}\n")?;
        std::fs::write(
            codex_home.join("auth.json"),
            r#"{"access_token":"fixture-token"}"#,
        )?;
        std::fs::write(
            codex_home.join(".credentials.json"),
            r#"{"fixture-server":{}}"#,
        )?;

        let parent_rollout = codex_home.join("sessions/2026/01/02/rollout-parent.jsonl");
        std::fs::write(
            &parent_rollout,
            format!(
                "{}\n",
                json!({
                    "type": "session_meta",
                    "payload": {
                        "id": PARENT_ID,
                        "session_id": PARENT_ID,
                    },
                })
            ),
        )?;
        let target_rollout = codex_home.join("sessions/2026/01/02/rollout-target.jsonl");
        let target_meta = format!(
            "{}\n",
            json!({
                "type": "session_meta",
                "payload": {
                    "id": THREAD_ID,
                    "session_id": THREAD_ID,
                    "forked_from_id": PARENT_ID,
                },
            })
        );
        let compressed_path = target_rollout.with_extension("jsonl.zst");
        std::fs::write(
            &compressed_path,
            zstd::stream::encode_all(target_meta.as_bytes(), 0)?,
        )?;

        let state_path = sqlite_home.join(super::database::STATE_DB_FILENAME);
        let state = Connection::open(state_path)?;
        state.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE threads (
                 id TEXT PRIMARY KEY,
                 rollout_path TEXT NOT NULL,
                 history_mode TEXT NOT NULL
             );",
        )?;
        state.execute(
            "INSERT INTO threads (id, rollout_path, history_mode) VALUES (?1, ?2, ?3)",
            (PARENT_ID, parent_rollout.to_str(), "legacy"),
        )?;
        state.execute(
            "INSERT INTO threads (id, rollout_path, history_mode) VALUES (?1, ?2, ?3)",
            (THREAD_ID, target_rollout.to_str(), "paginated"),
        )?;

        let thread_history_path = sqlite_home.join(super::database::THREAD_HISTORY_DB_FILENAME);
        let thread_history = Connection::open(thread_history_path)?;
        thread_history.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE thread_turns (
                 thread_id TEXT NOT NULL,
                 turn_id TEXT NOT NULL,
                 status TEXT NOT NULL,
                 PRIMARY KEY (thread_id, turn_id)
             );
             CREATE TABLE thread_items (
                 thread_id TEXT NOT NULL,
                 turn_id TEXT NOT NULL,
                 item_id TEXT NOT NULL,
                 rollout_ordinal INTEGER NOT NULL,
                 item_json TEXT NOT NULL,
                 PRIMARY KEY (thread_id, turn_id, item_id)
             );",
        )?;
        thread_history.execute(
            "INSERT INTO thread_turns (thread_id, turn_id, status) VALUES (?1, ?2, ?3)",
            (THREAD_ID, TURN_ID, "completed"),
        )?;
        thread_history.execute(
            "INSERT INTO thread_items
                (thread_id, turn_id, item_id, rollout_ordinal, item_json)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            (
                THREAD_ID,
                TURN_ID,
                "fixture-final-item",
                1,
                json!({
                    "id": "fixture-final-item",
                    "type": "agentMessage",
                    "text": FINAL_MESSAGE,
                })
                .to_string(),
            ),
        )?;

        Ok(Fixture {
            _root: root,
            source: SourcePaths {
                codex_home,
                sqlite_home,
            },
            _state: state,
            _thread_history: thread_history,
            target_rollout,
            parent_rollout,
        })
    }

    fn thread() -> CodexThreadMetadata {
        CodexThreadMetadata {
            model_provider: "openai".to_string(),
            model: "fixture-model".to_string(),
            session_id: THREAD_ID.to_string(),
            turn_id: TURN_ID.to_string(),
            last_assistant_message: FINAL_MESSAGE.to_string(),
        }
    }

    fn file_mode(path: &Path) -> Result<Option<u32>> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            Ok(Some(std::fs::metadata(path)?.permissions().mode() & 0o777))
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Ok(None)
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct PrivateHomeObservation {
        target_rollout_path: String,
        parent_rollout_path: String,
        target_turn_status: String,
        target_plain_history_exists: bool,
        target_compressed_history_exists: bool,
        parent_history_exists: bool,
        auth_snapshot: String,
        mcp_credentials_snapshot: String,
        target_compressed_bytes: Vec<u8>,
        parent_history_bytes: Vec<u8>,
        auth_file_mode: Option<u32>,
        source_plain_history_exists: bool,
        source_compressed_history_exists: bool,
        source_parent_history_exists: bool,
    }

    #[test]
    fn prepares_an_isolated_home_with_online_database_backups_and_private_lineage() -> Result<()> {
        let fixture = fixture()?;
        let inference_dir = fixture._root.path().join("inference");
        std::fs::create_dir_all(&inference_dir)?;
        let inference_dir = inference_dir.canonicalize()?;
        let private_home = prepare_from_source(&inference_dir, &fixture.source, &thread())?;
        let private_target = private_home.codex_home.join(
            fixture
                .target_rollout
                .strip_prefix(&fixture.source.codex_home)?,
        );
        let private_parent = private_home.codex_home.join(
            fixture
                .parent_rollout
                .strip_prefix(&fixture.source.codex_home)?,
        );
        let copied_state = Connection::open(
            private_home
                .sqlite_home
                .join(super::database::STATE_DB_FILENAME),
        )?;
        let copied_history = Connection::open(
            private_home
                .sqlite_home
                .join(super::database::THREAD_HISTORY_DB_FILENAME),
        )?;
        let target_path: String = copied_state.query_row(
            "SELECT rollout_path FROM threads WHERE id = ?1",
            [THREAD_ID],
            |row| row.get(0),
        )?;
        let parent_path: String = copied_state.query_row(
            "SELECT rollout_path FROM threads WHERE id = ?1",
            [PARENT_ID],
            |row| row.get(0),
        )?;
        let copied_auth = std::fs::read_to_string(private_home.codex_home.join("auth.json"))?;
        let copied_mcp_credentials =
            std::fs::read_to_string(private_home.codex_home.join(".credentials.json"))?;
        let copied_target_rollout = std::fs::read(private_target.with_extension("jsonl.zst"))?;
        let copied_parent_rollout = std::fs::read(&private_parent)?;
        let copied_auth_mode = file_mode(&private_home.codex_home.join("auth.json"))?;
        let target_turn_status: String = copied_history.query_row(
            "SELECT status FROM thread_turns WHERE thread_id = ?1 AND turn_id = ?2",
            (THREAD_ID, TURN_ID),
            |row| row.get(0),
        )?;

        assert_eq!(
            PrivateHomeObservation {
                target_rollout_path: target_path,
                parent_rollout_path: parent_path,
                target_turn_status,
                target_plain_history_exists: private_target.exists(),
                target_compressed_history_exists: private_home
                    .codex_home
                    .join("sessions/2026/01/02/rollout-target.jsonl.zst")
                    .exists(),
                parent_history_exists: private_parent.exists(),
                auth_snapshot: copied_auth,
                mcp_credentials_snapshot: copied_mcp_credentials,
                target_compressed_bytes: copied_target_rollout,
                parent_history_bytes: copied_parent_rollout,
                auth_file_mode: copied_auth_mode,
                source_plain_history_exists: fixture.target_rollout.exists(),
                source_compressed_history_exists: fixture
                    .target_rollout
                    .with_extension("jsonl.zst")
                    .exists(),
                source_parent_history_exists: fixture.parent_rollout.exists(),
            },
            PrivateHomeObservation {
                target_rollout_path: private_target.to_string_lossy().into_owned(),
                parent_rollout_path: private_parent.to_string_lossy().into_owned(),
                target_turn_status: "completed".to_string(),
                target_plain_history_exists: false,
                target_compressed_history_exists: true,
                parent_history_exists: true,
                auth_snapshot: r#"{"access_token":"fixture-token"}"#.to_string(),
                mcp_credentials_snapshot: r#"{"fixture-server":{}}"#.to_string(),
                target_compressed_bytes: std::fs::read(
                    fixture.target_rollout.with_extension("jsonl.zst"),
                )?,
                parent_history_bytes: std::fs::read(&fixture.parent_rollout)?,
                auth_file_mode: if cfg!(unix) { Some(0o600) } else { None },
                source_plain_history_exists: false,
                source_compressed_history_exists: true,
                source_parent_history_exists: true,
            },
        );
        Ok(())
    }

    #[derive(Clone, Copy)]
    enum InvalidSnapshot {
        NonPaginatedTarget,
        FinalMessageMismatch,
        UnsupportedRolloutDirectory,
    }

    #[rstest]
    #[case::non_paginated_target(
        InvalidSnapshot::NonPaginatedTarget,
        "Codex status fork target history is not paginated"
    )]
    #[case::final_message_mismatch(
        InvalidSnapshot::FinalMessageMismatch,
        "target final assistant message does not match private Codex history backup"
    )]
    #[case::unsupported_rollout_directory(
        InvalidSnapshot::UnsupportedRolloutDirectory,
        "Codex rollout history is outside its managed directories"
    )]
    fn rejects_inconsistent_private_history(
        #[case] invalid_snapshot: InvalidSnapshot,
        #[case] expected_error: &str,
    ) -> Result<()> {
        let fixture = fixture()?;
        let inference_dir = fixture._root.path().join("rejected-inference");
        std::fs::create_dir_all(&inference_dir)?;
        let inference_dir = inference_dir.canonicalize()?;
        let mut thread = thread();
        match invalid_snapshot {
            InvalidSnapshot::NonPaginatedTarget => {
                fixture._state.execute(
                    "UPDATE threads SET history_mode = ?1 WHERE id = ?2",
                    ("legacy", THREAD_ID),
                )?;
            }
            InvalidSnapshot::FinalMessageMismatch => {
                thread.last_assistant_message = "A different fixture response.".to_string();
            }
            InvalidSnapshot::UnsupportedRolloutDirectory => {
                let unsupported_rollout = fixture.source.codex_home.join("external/rollout.jsonl");
                std::fs::create_dir_all(
                    unsupported_rollout
                        .parent()
                        .expect("fixture rollout path should have a parent"),
                )?;
                std::fs::write(
                    &unsupported_rollout,
                    format!(
                        "{}\n",
                        json!({"type":"session_meta","payload":{"id":THREAD_ID}})
                    ),
                )?;
                fixture._state.execute(
                    "UPDATE threads SET rollout_path = ?1 WHERE id = ?2",
                    (unsupported_rollout.to_str(), THREAD_ID),
                )?;
            }
        }

        let actual = prepare_from_source(&inference_dir, &fixture.source, &thread)
            .map(|_| ())
            .map_err(|error| format!("{error:#}"));
        assert_eq!(actual, Err(expected_error.to_string()),);
        Ok(())
    }
}
