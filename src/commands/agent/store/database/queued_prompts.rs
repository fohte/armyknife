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
    pub(crate) action_event_kind: Option<WatchWebActionEventKind>,
    pub(crate) attachments: Vec<WatchWebQueuedAttachment>,
    pub(crate) resolved: bool,
    pub(crate) sent_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WatchWebActionEventKind {
    VibeModeOn,
    VibeModeOff,
    TodoCompleted,
    TodoRejected,
    DecisionChange,
}

impl WatchWebActionEventKind {
    fn as_db_value(self) -> &'static str {
        match self {
            Self::VibeModeOn => "vibe_mode_on",
            Self::VibeModeOff => "vibe_mode_off",
            Self::TodoCompleted => "todo_completed",
            Self::TodoRejected => "todo_rejected",
            Self::DecisionChange => "decision_change",
        }
    }

    fn from_db_value(value: Option<String>) -> Result<Option<Self>> {
        let Some(value) = value else {
            return Ok(None);
        };
        match value.as_str() {
            "vibe_mode_on" => Ok(Some(Self::VibeModeOn)),
            "vibe_mode_off" => Ok(Some(Self::VibeModeOff)),
            "todo_completed" => Ok(Some(Self::TodoCompleted)),
            "todo_rejected" => Ok(Some(Self::TodoRejected)),
            "decision_change" => Ok(Some(Self::DecisionChange)),
            _ => anyhow::bail!("unknown watch-web action event kind: {value}"),
        }
    }
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
    action_event_kind: Option<WatchWebActionEventKind>,
    sent_at: DateTime<Utc>,
) -> Result<i64> {
    super::validate_session_id(session_id)?;
    let connection = open_connection(sessions_dir)?;
    let attachments = serde_json::to_string(attachments)?;
    connection.execute(
        "INSERT INTO watch_web_queued_prompt (
            session_id, message_id, content, attachments, action_event_kind, sent_at, resolved
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
        params![
            session_id,
            message_id,
            content,
            attachments,
            action_event_kind.map(WatchWebActionEventKind::as_db_value),
            sent_at.to_rfc3339()
        ],
    )?;
    Ok(connection.last_insert_rowid())
}

pub(crate) fn move_watch_web_queued_prompts_to_session(
    sessions_dir: &Path,
    launch_session_id: &str,
    session_id: &str,
) -> Result<()> {
    super::validate_session_id(launch_session_id)?;
    super::validate_session_id(session_id)?;
    if !sessions_dir.exists() {
        return Ok(());
    }

    let connection = open_connection(sessions_dir)?;
    connection.execute(
        "UPDATE watch_web_queued_prompt SET session_id = ?2 WHERE session_id = ?1",
        params![launch_session_id, session_id],
    )?;
    Ok(())
}

pub(crate) fn watch_web_queued_prompt_message_id_exists(
    sessions_dir: &Path,
    session_id: &str,
    message_id: &str,
) -> Result<bool> {
    super::validate_session_id(session_id)?;
    if !sessions_dir.exists() {
        return Ok(false);
    }

    let connection = open_connection(sessions_dir)?;
    connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM watch_web_queued_prompt
                WHERE session_id = ?1 AND message_id = ?2
            )",
            params![session_id, message_id],
            |row| row.get(0),
        )
        .map_err(Into::into)
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
        "SELECT id, message_id, content, attachments, action_event_kind, resolved, sent_at
         FROM watch_web_queued_prompt
         WHERE session_id = ?1",
    )?;
    let rows = statement.query_map([session_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, bool>(5)?,
            row.get::<_, String>(6)?,
        ))
    })?;
    let mut prompts = rows
        .map(|row| {
            let (id, message_id, content, attachments, action_event_kind, resolved, sent_at) = row?;
            Ok(WatchWebQueuedPrompt {
                id,
                message_id,
                content,
                action_event_kind: WatchWebActionEventKind::from_db_value(action_event_kind)?,
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
        WatchWebActionEventKind, WatchWebQueuedAttachment, WatchWebQueuedPrompt,
        delete_watch_web_queued_prompts, insert_watch_web_queued_prompt,
        mark_watch_web_queued_prompts_resolved, watch_web_queued_prompt_message_id_exists,
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
            None,
            first_sent_at,
        )
        .unwrap();
        let second_id = insert_watch_web_queued_prompt(
            sessions_dir.path(),
            "fixture-session-a",
            "22222222-2222-4222-8222-222222222222",
            "Second queued message",
            &[],
            None,
            second_sent_at,
        )
        .unwrap();
        insert_watch_web_queued_prompt(
            sessions_dir.path(),
            "fixture-session-b",
            "33333333-3333-4333-8333-333333333333",
            "Another session's queued message",
            &[],
            None,
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
                        action_event_kind: None,
                        attachments: vec![],
                        resolved: false,
                        sent_at: first_sent_at,
                    },
                    WatchWebQueuedPrompt {
                        id: second_id,
                        message_id: Some("22222222-2222-4222-8222-222222222222".to_string()),
                        content: "Second queued message".to_string(),
                        action_event_kind: None,
                        attachments: vec![],
                        resolved: false,
                        sent_at: second_sent_at,
                    },
                ],
                vec![WatchWebQueuedPrompt {
                    id: second_id,
                    message_id: Some("22222222-2222-4222-8222-222222222222".to_string()),
                    content: "Second queued message".to_string(),
                    action_event_kind: None,
                    attachments: vec![],
                    resolved: false,
                    sent_at: second_sent_at,
                }],
                vec![WatchWebQueuedPrompt {
                    id: second_id + 1,
                    message_id: Some("33333333-3333-4333-8333-333333333333".to_string()),
                    content: "Another session's queued message".to_string(),
                    action_event_kind: None,
                    attachments: vec![],
                    resolved: false,
                    sent_at: second_sent_at,
                }],
            ),
        );
    }

    #[rstest]
    fn detects_reused_message_ids_within_a_session(sessions_dir: TempDir) {
        let sent_at = chrono::Utc
            .with_ymd_and_hms(2026, 3, 14, 9, 26, 53)
            .unwrap();
        let message_id = "88888888-8888-4888-8888-888888888888";
        let before_insert = watch_web_queued_prompt_message_id_exists(
            sessions_dir.path(),
            "fixture-session-a",
            message_id,
        )
        .unwrap();
        insert_watch_web_queued_prompt(
            sessions_dir.path(),
            "fixture-session-a",
            message_id,
            "A queued message",
            &[],
            None,
            sent_at,
        )
        .unwrap();
        let after_insert = watch_web_queued_prompt_message_id_exists(
            sessions_dir.path(),
            "fixture-session-a",
            message_id,
        )
        .unwrap();
        let other_session = watch_web_queued_prompt_message_id_exists(
            sessions_dir.path(),
            "fixture-session-b",
            message_id,
        )
        .unwrap();

        assert_eq!(
            (before_insert, after_insert, other_session),
            (false, true, false)
        );
    }

    #[rstest]
    #[case::vibe_mode_on(WatchWebActionEventKind::VibeModeOn)]
    #[case::vibe_mode_off(WatchWebActionEventKind::VibeModeOff)]
    #[case::todo_completed(WatchWebActionEventKind::TodoCompleted)]
    #[case::todo_rejected(WatchWebActionEventKind::TodoRejected)]
    #[case::decision_change(WatchWebActionEventKind::DecisionChange)]
    fn round_trips_action_event_kind(
        sessions_dir: TempDir,
        #[case] action_event_kind: WatchWebActionEventKind,
    ) {
        let sent_at = chrono::Utc
            .with_ymd_and_hms(2026, 3, 14, 9, 26, 53)
            .unwrap();
        let id = insert_watch_web_queued_prompt(
            sessions_dir.path(),
            "fixture-session-a",
            "fixture-action-message",
            "Recorded action event text",
            &[],
            Some(action_event_kind),
            sent_at,
        )
        .unwrap();

        assert_eq!(
            watch_web_queued_prompts(sessions_dir.path(), "fixture-session-a").unwrap(),
            vec![WatchWebQueuedPrompt {
                id,
                message_id: Some("fixture-action-message".to_owned()),
                content: "Recorded action event text".to_owned(),
                action_event_kind: Some(action_event_kind),
                attachments: vec![],
                resolved: false,
                sent_at,
            }],
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
            None,
            later_sent_at,
        )
        .unwrap();
        let earlier_id = insert_watch_web_queued_prompt(
            sessions_dir.path(),
            "fixture-session-a",
            "55555555-5555-4555-8555-555555555555",
            "Earlier dispatched request",
            &[],
            None,
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
                    action_event_kind: None,
                    attachments: vec![],
                    resolved: false,
                    sent_at: earlier_sent_at,
                },
                WatchWebQueuedPrompt {
                    id: later_id,
                    message_id: Some("44444444-4444-4444-8444-444444444444".to_string()),
                    content: "Later dispatched request".to_string(),
                    action_event_kind: None,
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
            None,
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
                action_event_kind: None,
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
            None,
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
