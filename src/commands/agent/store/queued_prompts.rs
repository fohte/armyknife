use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, Utc};

use super::database::{self, WatchWebQueuedAttachment, WatchWebQueuedPrompt};

pub(crate) fn ensure_watch_web_user_marker_from(
    sessions_dir: &Path,
    session_id: &str,
) -> Result<String> {
    if let Some(marker) = super::watch_web_user_marker_from(sessions_dir, session_id)? {
        return Ok(marker);
    }
    database::lock_session_for_update(sessions_dir, session_id)?.ensure_watch_web_user_marker()
}

pub(crate) fn insert_watch_web_queued_prompt_from(
    sessions_dir: &Path,
    session_id: &str,
    message_id: &str,
    content: &str,
    attachments: &[WatchWebQueuedAttachment],
    sent_at: DateTime<Utc>,
) -> Result<i64> {
    database::insert_watch_web_queued_prompt(
        sessions_dir,
        session_id,
        message_id,
        content,
        attachments,
        sent_at,
    )
}

pub(crate) fn delete_watch_web_queued_prompts_from(
    sessions_dir: &Path,
    session_id: &str,
    ids: &[i64],
) -> Result<()> {
    database::delete_watch_web_queued_prompts(sessions_dir, session_id, ids)
}

pub(crate) fn watch_web_queued_prompts_from(
    sessions_dir: &Path,
    session_id: &str,
) -> Result<Vec<WatchWebQueuedPrompt>> {
    database::watch_web_queued_prompts(sessions_dir, session_id)
}

pub(crate) fn mark_watch_web_queued_prompts_resolved(session_id: &str, ids: &[i64]) -> Result<()> {
    database::mark_watch_web_queued_prompts_resolved(&super::sessions_dir()?, session_id, ids)
}
