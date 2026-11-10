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
    let sessions_dir = if session.engine == Engine::Claude {
        Some(resolve_sessions_dir(state.sessions_dir.clone()).map_err(ApiError::internal)?)
    } else {
        None
    };
    let marker = if let Some(sessions_dir) = sessions_dir.as_ref() {
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
    let attachment_paths =
        run_blocking(move || attachments::save(attachments, state.attachment_dir)).await?;
    let content = attachments::append_paths_to_message(request.content, &attachment_paths);
    let queued_content = content.clone();
    let content = watch_web_message_content(&session, marker.as_deref(), &content);
    let cleanup_paths = attachment_paths;
    let runtime = state.runtime;
    run_blocking(move || {
        let sent_at = Utc::now();
        let result = runtime.send_message(&session, &content);
        if result.is_err() {
            attachments::remove(&cleanup_paths);
        } else if let Some(sessions_dir) = sessions_dir.as_ref()
            && let Err(error) = store::insert_watch_web_queued_prompt_from(
                sessions_dir,
                &session.session_id,
                &queued_content,
                sent_at,
            )
        {
            tracing::warn!(
                    session_id = %session.session_id,
                %error,
                "failed to save sent watch-web queued prompt"
            );
        }
        result
    })
    .await?;
    Ok(Json(Success::new()))
}

fn watch_web_message_content(session: &Session, marker: Option<&str>, content: &str) -> String {
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
