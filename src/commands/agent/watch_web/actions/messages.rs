use anyhow::Result;
use axum::Json;
use axum::extract::{Path as RoutePath, State};
use chrono::Utc;
use serde::Deserialize;

use super::super::super::{
    peer::notify,
    store,
    types::{Engine, Session, SessionStatus},
};
use super::super::attachments;
use super::{ActionState, ApiError, Success, load_session, resolve_sessions_dir, run_blocking};

pub(super) const MAX_MESSAGE_BODY_BYTES: usize = 72 * 1024 * 1024;

pub(super) fn send_live_message(session: &Session, content: &str) -> Result<()> {
    send_via_peer_notify(&session.session_id, content, |session_id, content| {
        notify::notify(session_id, content, None, None)
    })
}

#[derive(Deserialize)]
pub(super) struct MessageRequest {
    content: String,
    #[serde(default)]
    attachments: Vec<attachments::Request>,
    #[serde(default)]
    images: Vec<attachments::LegacyImageRequest>,
}

pub(super) async fn send_message(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
    Json(request): Json<MessageRequest>,
) -> Result<Json<Success>, ApiError> {
    let session = load_session(state.clone(), id).await?;
    if session.status == SessionStatus::Ended {
        return Err(ApiError::conflict("session has ended"));
    }
    if request.content.trim().is_empty()
        && request.attachments.is_empty()
        && request.images.is_empty()
    {
        return Err(ApiError::bad_request("message must not be empty"));
    }

    let attachments = attachments::validate(request.attachments, request.images)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let sessions_dir =
        resolve_sessions_dir(state.sessions_dir.clone()).map_err(ApiError::internal)?;
    let marker = if session.engine == Engine::Claude {
        let session_id = session.session_id.clone();
        let sessions_dir = sessions_dir.clone();
        Some(
            run_blocking(move || {
                store::ensure_watch_web_user_marker_from(&sessions_dir, &session_id)
            })
            .await?,
        )
    } else {
        None
    };
    let saved_attachments =
        run_blocking(move || attachments::save(attachments, state.attachment_dir)).await?;
    let attachment_paths = saved_attachments
        .iter()
        .map(|attachment| attachment.path.clone())
        .collect::<Vec<_>>();
    let content = attachments::append_paths_to_message(request.content.clone(), &attachment_paths);
    let queued_attachments = saved_attachments
        .iter()
        .map(|attachment| match &attachment.image_source {
            Some(source) => store::WatchWebQueuedAttachment::Image {
                source: source.clone(),
            },
            None => store::WatchWebQueuedAttachment::File {
                name: attachment.name.clone(),
            },
        })
        .collect::<Vec<_>>();
    let message_id = uuid::Uuid::new_v4().to_string();
    let sent_at = Utc::now();
    let queued_prompt_id = run_blocking({
        let sessions_dir = sessions_dir.clone();
        let session_id = session.session_id.clone();
        let message_id = message_id.clone();
        let content = request.content;
        move || {
            store::insert_watch_web_queued_prompt_from(
                &sessions_dir,
                &session_id,
                &message_id,
                &content,
                &queued_attachments,
                sent_at,
            )
        }
    })
    .await;
    let queued_prompt_id = match queued_prompt_id {
        Ok(id) => id,
        Err(error) => {
            let cleanup_paths = attachment_paths;
            if let Err(cleanup_error) = run_blocking(move || {
                attachments::remove(&cleanup_paths);
                Ok(())
            })
            .await
            {
                tracing::warn!(
                    error = %cleanup_error.message,
                    "failed to remove unsent watch-web attachments"
                );
            }
            return Err(error);
        }
    };
    let content = watch_web_message_content(&session, marker.as_deref(), &content);
    let content = super::super::messages::queued_prompt::add_id_marker(&content, &message_id);
    let cleanup_paths = attachment_paths;
    let runtime = state.runtime;
    let send_session = session.clone();
    let send_result = run_blocking(move || runtime.send_message(&send_session, &content)).await;
    if let Err(error) = send_result {
        let cleanup_session_id = session.session_id.clone();
        let cleanup_result = run_blocking(move || {
            attachments::remove(&cleanup_paths);
            store::delete_watch_web_queued_prompts_from(
                &sessions_dir,
                &cleanup_session_id,
                &[queued_prompt_id],
            )
        })
        .await;
        if let Err(cleanup_error) = cleanup_result {
            tracing::warn!(
                session_id = %session.session_id,
                error = %cleanup_error.message,
                "failed to remove unsent watch-web queued prompt"
            );
        }
        return Err(error);
    }
    Ok(Json(Success::new()))
}

pub(super) fn watch_web_message_content(
    session: &Session,
    marker: Option<&str>,
    content: &str,
) -> String {
    match (session.engine, marker) {
        (Engine::Claude, Some(marker)) => {
            super::super::super::watch_web_user_marker::prefix_message(marker, content)
        }
        _ => content.to_string(),
    }
}
fn send_via_peer_notify(
    session_id: &str,
    content: &str,
    deliver: impl FnOnce(&str, &str) -> Result<notify::Delivery>,
) -> Result<()> {
    deliver(session_id, content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::send_via_peer_notify;
    use crate::commands::agent::peer::notify;

    #[test]
    fn live_message_adapter_forwards_session_and_content_to_peer_notify() {
        let mut calls = Vec::new();

        let result = send_via_peer_notify(
            "sample-session",
            "continue the investigation",
            |session_id, content| {
                calls.push((session_id.to_string(), content.to_string()));
                Ok(notify::Delivery::Sent)
            },
        );

        assert_eq!(
            (result.map_err(|error| error.to_string()), calls),
            (
                Ok(()),
                vec![(
                    "sample-session".to_string(),
                    "continue the investigation".to_string(),
                )],
            ),
        );
    }
}
