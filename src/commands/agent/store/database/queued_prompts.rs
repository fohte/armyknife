use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{TransactionBehavior, params};

use super::open_connection;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WatchWebQueuedPrompt {
    pub(crate) id: i64,
    pub(crate) content: String,
    pub(crate) sent_at: DateTime<Utc>,
}

pub(crate) fn insert_watch_web_queued_prompt(
    sessions_dir: &Path,
    session_id: &str,
    content: &str,
    sent_at: DateTime<Utc>,
) -> Result<i64> {
    super::validate_session_id(session_id)?;
    let connection = open_connection(sessions_dir)?;
    connection.execute(
        "INSERT INTO watch_web_queued_prompt (session_id, content, sent_at)
         VALUES (?1, ?2, ?3)",
        params![session_id, content, sent_at.to_rfc3339()],
    )?;
    Ok(connection.last_insert_rowid())
}

pub(crate) fn watch_web_queued_prompts(
    sessions_dir: &Path,
    session_id: &str,
) -> Result<Vec<WatchWebQueuedPrompt>> {
    super::validate_session_id(session_id)?;
    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }

    let connection = open_connection(sessions_dir)?;
    let mut statement = connection.prepare(
        "SELECT id, content, sent_at
         FROM watch_web_queued_prompt
         WHERE session_id = ?1",
    )?;
    let rows = statement.query_map([session_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut prompts = rows
        .map(|row| {
            let (id, content, sent_at) = row?;
            Ok(WatchWebQueuedPrompt {
                id,
                content,
                sent_at: DateTime::parse_from_rfc3339(&sent_at)?.with_timezone(&Utc),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    prompts.sort_by(|left, right| {
        left.sent_at
            .cmp(&right.sent_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(prompts)
}

pub(crate) fn delete_watch_web_queued_prompts(
    sessions_dir: &Path,
    session_id: &str,
    ids: &[i64],
) -> Result<()> {
    super::validate_session_id(session_id)?;
    if ids.is_empty() || !sessions_dir.exists() {
        return Ok(());
    }

    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for id in ids {
        transaction.execute(
            "DELETE FROM watch_web_queued_prompt WHERE session_id = ?1 AND id = ?2",
            params![session_id, id],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::{
        WatchWebQueuedPrompt, delete_watch_web_queued_prompts, insert_watch_web_queued_prompt,
        watch_web_queued_prompts,
    };

    #[fixture]
    fn sessions_dir() -> TempDir {
        tempfile::tempdir().expect("temporary sessions directory should be created")
    }

    #[rstest]
    fn stores_prompts_per_session_and_deletes_selected_ids(sessions_dir: TempDir) {
        let first_sent_at = chrono::Utc
            .with_ymd_and_hms(2026, 3, 14, 9, 26, 53)
            .unwrap();
        let second_sent_at = chrono::Utc
            .with_ymd_and_hms(2026, 3, 14, 9, 27, 53)
            .unwrap();
        let first_id = insert_watch_web_queued_prompt(
            sessions_dir.path(),
            "fixture-session-a",
            "First queued message with /fixture/image-a.webp",
            first_sent_at,
        )
        .unwrap();
        let second_id = insert_watch_web_queued_prompt(
            sessions_dir.path(),
            "fixture-session-a",
            "Second queued message",
            second_sent_at,
        )
        .unwrap();
        insert_watch_web_queued_prompt(
            sessions_dir.path(),
            "fixture-session-b",
            "Another session's queued message",
            second_sent_at,
        )
        .unwrap();

        let before_delete =
            watch_web_queued_prompts(sessions_dir.path(), "fixture-session-a").unwrap();
        delete_watch_web_queued_prompts(sessions_dir.path(), "fixture-session-a", &[first_id])
            .unwrap();
        let after_delete =
            watch_web_queued_prompts(sessions_dir.path(), "fixture-session-a").unwrap();
        let other_session =
            watch_web_queued_prompts(sessions_dir.path(), "fixture-session-b").unwrap();

        assert_eq!(
            (before_delete, after_delete, other_session),
            (
                vec![
                    WatchWebQueuedPrompt {
                        id: first_id,
                        content: "First queued message with /fixture/image-a.webp".to_string(),
                        sent_at: first_sent_at,
                    },
                    WatchWebQueuedPrompt {
                        id: second_id,
                        content: "Second queued message".to_string(),
                        sent_at: second_sent_at,
                    },
                ],
                vec![WatchWebQueuedPrompt {
                    id: second_id,
                    content: "Second queued message".to_string(),
                    sent_at: second_sent_at,
                }],
                vec![WatchWebQueuedPrompt {
                    id: second_id + 1,
                    content: "Another session's queued message".to_string(),
                    sent_at: second_sent_at,
                }],
            ),
        );
    }

    #[rstest]
    fn lists_prompts_by_send_time_when_inserts_finish_out_of_order(sessions_dir: TempDir) {
        let earlier_sent_at = chrono::Utc
            .with_ymd_and_hms(2026, 3, 14, 9, 26, 53)
            .unwrap();
        let later_sent_at = chrono::Utc
            .with_ymd_and_hms(2026, 3, 14, 9, 27, 53)
            .unwrap();
        let later_id = insert_watch_web_queued_prompt(
            sessions_dir.path(),
            "fixture-session-a",
            "Later dispatched request",
            later_sent_at,
        )
        .unwrap();
        let earlier_id = insert_watch_web_queued_prompt(
            sessions_dir.path(),
            "fixture-session-a",
            "Earlier dispatched request",
            earlier_sent_at,
        )
        .unwrap();

        assert_eq!(
            watch_web_queued_prompts(sessions_dir.path(), "fixture-session-a").unwrap(),
            vec![
                WatchWebQueuedPrompt {
                    id: earlier_id,
                    content: "Earlier dispatched request".to_string(),
                    sent_at: earlier_sent_at,
                },
                WatchWebQueuedPrompt {
                    id: later_id,
                    content: "Later dispatched request".to_string(),
                    sent_at: later_sent_at,
                },
            ],
        );
    }

    #[rstest]
    fn deleting_a_session_clears_its_queued_prompts(sessions_dir: TempDir) {
        let session: crate::commands::agent::types::Session =
            serde_json::from_value(serde_json::json!({
                "session_id": "fixture-session-a",
                "cwd": "/fixture/worktree",
                "transcript_path": null,
                "tmux_info": null,
                "status": "stopped",
                "created_at": "2026-03-14T09:26:53Z",
                "updated_at": "2026-03-14T09:26:53Z",
                "last_message": null,
            }))
            .unwrap();
        let sent_at = chrono::Utc
            .with_ymd_and_hms(2026, 3, 14, 9, 27, 53)
            .unwrap();
        crate::commands::agent::store::save_session_to(sessions_dir.path(), &session).unwrap();
        insert_watch_web_queued_prompt(
            sessions_dir.path(),
            &session.session_id,
            "Queued until this session is deleted",
            sent_at,
        )
        .unwrap();

        crate::commands::agent::store::delete_session_from(
            sessions_dir.path(),
            &session.session_id,
        )
        .unwrap();

        assert_eq!(
            watch_web_queued_prompts(sessions_dir.path(), &session.session_id).unwrap(),
            Vec::<WatchWebQueuedPrompt>::new(),
        );
    }
}
