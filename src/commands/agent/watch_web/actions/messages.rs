use std::{
    path::PathBuf,
    sync::{Arc, LazyLock},
};

use anyhow::Result;
use axum::Json;
use axum::extract::{Path as RoutePath, State};
use chrono::Utc;
use serde::Deserialize;
use tokio::sync::OwnedMutexGuard;

use super::super::super::{
    peer::notify,
    store,
    types::{Engine, Session, SessionStatus},
};
use super::super::attachments;
use super::keyed_mutex_registry::KeyedMutexRegistry;
use super::{ActionState, ApiError, Success, load_session, resolve_sessions_dir, run_blocking};

pub(super) const MAX_MESSAGE_BODY_BYTES: usize = 72 * 1024 * 1024;

static SESSION_ACTION_LOCKS: LazyLock<KeyedMutexRegistry<String>> =
    LazyLock::new(KeyedMutexRegistry::default);

pub(super) async fn lock_session_action(session_id: &str) -> OwnedMutexGuard<()> {
    SESSION_ACTION_LOCKS
        .get_or_create(session_id.to_owned())
        .await
        .lock_owned()
        .await
}

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
    let _session_action_guard = lock_session_action(&id).await;
    let session = load_session(state.clone(), id).await?;
    if session.sweep_signaled {
        return Err(ApiError::conflict("session is being stopped"));
    }
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
    let queued_content = request.content;
    let content = attachments::append_paths_to_message(queued_content.clone(), &attachment_paths);
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
    send_queued_prompt(QueuedPromptDelivery {
        runtime: state.runtime,
        sessions_dir,
        session,
        marker,
        queued_content,
        message_content: content,
        queued_attachments,
        attachment_paths,
    })
    .await?;
    Ok(Json(Success::new()))
}

pub(super) struct QueuedPromptDelivery {
    pub(super) runtime: Arc<dyn super::ActionRuntime>,
    pub(super) sessions_dir: PathBuf,
    pub(super) session: Session,
    pub(super) marker: Option<String>,
    pub(super) queued_content: String,
    pub(super) message_content: String,
    pub(super) queued_attachments: Vec<store::WatchWebQueuedAttachment>,
    pub(super) attachment_paths: Vec<PathBuf>,
}

pub(super) async fn send_queued_prompt(delivery: QueuedPromptDelivery) -> Result<(), ApiError> {
    let QueuedPromptDelivery {
        runtime,
        sessions_dir,
        session,
        marker,
        queued_content,
        message_content,
        queued_attachments,
        attachment_paths,
    } = delivery;
    let message_id = uuid::Uuid::new_v4().to_string();
    let sent_at = Utc::now();
    let queued_prompt_id = run_blocking({
        let sessions_dir = sessions_dir.clone();
        let session_id = session.session_id.clone();
        let message_id = message_id.clone();
        move || {
            store::insert_watch_web_queued_prompt_from(
                &sessions_dir,
                &session_id,
                &message_id,
                &queued_content,
                &queued_attachments,
                sent_at,
            )
        }
    })
    .await;
    let queued_prompt_id = match queued_prompt_id {
        Ok(id) => id,
        Err(error) => {
            remove_attachments(attachment_paths).await;
            return Err(error);
        }
    };

    let content = watch_web_message_content(&session, marker.as_deref(), &message_content);
    let content = super::super::messages::queued_prompt::add_id_marker(&content, &message_id);
    let send_session = session.clone();
    if let Err(error) = run_blocking(move || runtime.send_message(&send_session, &content)).await {
        let cleanup_session_id = session.session_id.clone();
        let delete_result = run_blocking({
            let sessions_dir = sessions_dir.clone();
            let cleanup_session_id = cleanup_session_id.clone();
            move || {
                store::delete_watch_web_queued_prompts_from(
                    &sessions_dir,
                    &cleanup_session_id,
                    &[queued_prompt_id],
                )
            }
        })
        .await;
        let row_cleaned = if let Err(delete_error) = delete_result {
            tracing::warn!(
                session_id = %cleanup_session_id,
                error = %delete_error.message,
                "failed to remove unsent watch-web queued prompt"
            );
            match run_blocking(move || {
                store::mark_watch_web_queued_prompts_resolved_from(
                    &sessions_dir,
                    &cleanup_session_id,
                    &[queued_prompt_id],
                )
            })
            .await
            {
                Ok(()) => true,
                Err(mark_error) => {
                    tracing::warn!(
                        session_id = %session.session_id,
                        error = %mark_error.message,
                        "failed to resolve unsent watch-web queued prompt"
                    );
                    false
                }
            }
        } else {
            true
        };
        if row_cleaned {
            remove_attachments(attachment_paths).await;
        } else {
            tracing::warn!(
                session_id = %session.session_id,
                "unsent watch-web queued prompt remains unresolved"
            );
        }
        return Err(error);
    }
    Ok(())
}

async fn remove_attachments(paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    if let Err(error) = run_blocking(move || {
        attachments::remove(&paths);
        Ok(())
    })
    .await
    {
        tracing::warn!(error = %error.message, "failed to remove unsent watch-web attachments");
    }
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
