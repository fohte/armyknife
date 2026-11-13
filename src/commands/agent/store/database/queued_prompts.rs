use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use super::open_connection;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WatchWebQueuedPrompt {
    pub(crate) id: i64,
    pub(crate) message_id: Option<String>,
    pub(crate) content: String,
    pub(crate) attachments: Vec<WatchWebQueuedAttachment>,
    pub(crate) resolved: bool,
    pub(crate) sent_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum WatchWebQueuedAttachment {
    Image { source: String },
    File { name: String },
}

pub(crate) fn insert_watch_web_queued_prompt(
    sessions_dir: &Path,
    session_id: &str,
    message_id: &str,
    content: &str,
    attachments: &[WatchWebQueuedAttachment],
    sent_at: DateTime<Utc>,
) -> Result<i64> {
    super::validate_session_id(session_id)?;
    let connection = open_connection(sessions_dir)?;
    let attachments = serde_json::to_string(attachments)?;
    connection.execute(
        "INSERT INTO watch_web_queued_prompt (
            session_id, message_id, content, attachments, sent_at, resolved
         ) VALUES (?1, ?2, ?3, ?4, ?5, 0)",
        params![
            session_id,
            message_id,
            content,
            attachments,
            sent_at.to_rfc3339()
        ],
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
        "SELECT id, message_id, content, attachments, resolved, sent_at
         FROM watch_web_queued_prompt
         WHERE session_id = ?1",
    )?;
    let rows = statement.query_map([session_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, bool>(4)?,
            row.get::<_, String>(5)?,
        ))
    })?;
    let mut prompts = rows
        .map(|row| {
            let (id, message_id, content, attachments, resolved, sent_at) = row?;
            Ok(WatchWebQueuedPrompt {
                id,
                message_id,
                content,
                attachments: serde_json::from_str(&attachments)?,
                resolved,
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

pub(crate) fn mark_watch_web_queued_prompts_resolved(
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
            "UPDATE watch_web_queued_prompt SET resolved = 1 WHERE session_id = ?1 AND id = ?2",
            params![session_id, id],
        )?;
    }
    transaction.commit()?;
    Ok(())
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

    use super::super::peer_out_messages::{
        insert_watch_web_peer_out_message, watch_web_peer_out_messages,
    };
    use super::{
        WatchWebQueuedAttachment, WatchWebQueuedPrompt, delete_watch_web_queued_prompts,
        insert_watch_web_queued_prompt, mark_watch_web_queued_prompts_resolved,
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
            "11111111-1111-4111-8111-111111111111",
            "First queued message with /fixture/image-a.webp",
            &[],
            first_sent_at,
        )
        .unwrap();
        let second_id = insert_watch_web_queued_prompt(
            sessions_dir.path(),
            "fixture-session-a",
            "22222222-2222-4222-8222-222222222222",
            "Second queued message",
            &[],
            second_sent_at,
        )
        .unwrap();
        insert_watch_web_queued_prompt(
            sessions_dir.path(),
            "fixture-session-b",
            "33333333-3333-4333-8333-333333333333",
            "Another session's queued message",
            &[],
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
                        message_id: Some("11111111-1111-4111-8111-111111111111".to_string()),
                        content: "First queued message with /fixture/image-a.webp".to_string(),
                        attachments: vec![],
                        resolved: false,
                        sent_at: first_sent_at,
                    },
                    WatchWebQueuedPrompt {
                        id: second_id,
                        message_id: Some("22222222-2222-4222-8222-222222222222".to_string()),
                        content: "Second queued message".to_string(),
                        attachments: vec![],
                        resolved: false,
                        sent_at: second_sent_at,
                    },
                ],
                vec![WatchWebQueuedPrompt {
                    id: second_id,
                    message_id: Some("22222222-2222-4222-8222-222222222222".to_string()),
                    content: "Second queued message".to_string(),
                    attachments: vec![],
                    resolved: false,
                    sent_at: second_sent_at,
                }],
                vec![WatchWebQueuedPrompt {
                    id: second_id + 1,
                    message_id: Some("33333333-3333-4333-8333-333333333333".to_string()),
                    content: "Another session's queued message".to_string(),
                    attachments: vec![],
                    resolved: false,
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
            "44444444-4444-4444-8444-444444444444",
            "Later dispatched request",
            &[],
            later_sent_at,
        )
        .unwrap();
        let earlier_id = insert_watch_web_queued_prompt(
            sessions_dir.path(),
            "fixture-session-a",
            "55555555-5555-4555-8555-555555555555",
            "Earlier dispatched request",
            &[],
            earlier_sent_at,
        )
        .unwrap();

        assert_eq!(
            watch_web_queued_prompts(sessions_dir.path(), "fixture-session-a").unwrap(),
            vec![
                WatchWebQueuedPrompt {
                    id: earlier_id,
                    message_id: Some("55555555-5555-4555-8555-555555555555".to_string()),
                    content: "Earlier dispatched request".to_string(),
                    attachments: vec![],
                    resolved: false,
                    sent_at: earlier_sent_at,
                },
                WatchWebQueuedPrompt {
                    id: later_id,
                    message_id: Some("44444444-4444-4444-8444-444444444444".to_string()),
                    content: "Later dispatched request".to_string(),
                    attachments: vec![],
                    resolved: false,
                    sent_at: later_sent_at,
                },
            ],
        );
    }

    #[rstest]
    fn keeps_structured_attachments_after_resolving_a_prompt(sessions_dir: TempDir) {
        let sent_at = chrono::Utc
            .with_ymd_and_hms(2026, 3, 14, 9, 26, 53)
            .unwrap();
        let attachments = [WatchWebQueuedAttachment::File {
            name: "fixture-notes.txt".to_owned(),
        }];
        let id = insert_watch_web_queued_prompt(
            sessions_dir.path(),
            "fixture-session-a",
            "77777777-7777-4777-8777-777777777777",
            "Review the attached file.",
            &attachments,
            sent_at,
        )
        .unwrap();

        mark_watch_web_queued_prompts_resolved(sessions_dir.path(), "fixture-session-a", &[id])
            .unwrap();

        assert_eq!(
            watch_web_queued_prompts(sessions_dir.path(), "fixture-session-a").unwrap(),
            vec![WatchWebQueuedPrompt {
                id,
                message_id: Some("77777777-7777-4777-8777-777777777777".to_owned()),
                content: "Review the attached file.".to_owned(),
                attachments: attachments.to_vec(),
                resolved: true,
                sent_at,
            }],
        );
    }

    #[rstest]
    fn deleting_a_session_clears_its_queued_and_sent_messages(sessions_dir: TempDir) {
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
            "66666666-6666-4666-8666-666666666666",
            "Queued until this session is deleted",
            &[],
            sent_at,
        )
        .unwrap();
        insert_watch_web_peer_out_message(
            sessions_dir.path(),
            &session.session_id,
            "fixture-target-session",
            "Delivered before this session was deleted",
            sent_at,
            false,
        )
        .unwrap();

        crate::commands::agent::store::delete_session_from_with_storage(
            sessions_dir.path(),
            &session.session_id,
            Some(&sessions_dir.path().join("attachments")),
        )
        .unwrap();

        assert_eq!(
            (
                watch_web_queued_prompts(sessions_dir.path(), &session.session_id).unwrap(),
                watch_web_peer_out_messages(sessions_dir.path(), &session.session_id).unwrap(),
            ),
            (Vec::<WatchWebQueuedPrompt>::new(), Vec::new(),),
        );
    }
}
