use std::{
    future::Future,
    path::{Path, PathBuf},
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
use super::super::messages::{action_event, queued_prompt};
use super::keyed_mutex_registry::KeyedMutexRegistry;
use super::{
    ActionState, ApiError, load_session, load_session_start_generation, resolve_sessions_dir,
    run_blocking,
};

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
    #[serde(default, rename = "messageId")]
    message_id: Option<String>,
}

fn is_valid_message_id(message_id: &str) -> bool {
    matches!(uuid::Uuid::parse_str(message_id), Ok(parsed) if parsed.to_string() == message_id)
}

pub(super) async fn send_message(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
    Json(request): Json<MessageRequest>,
) -> Result<Json<MessageSuccess>, ApiError> {
    let _session_action_guard = lock_session_action(&id).await;
    let session_start_generation = load_session_start_generation(&state, &id).await?;
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

    let message_id = request
        .message_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if !is_valid_message_id(&message_id) {
        return Err(ApiError::bad_request("invalid message ID"));
    }

    let attachments = attachments::validate(request.attachments, request.images)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let sessions_dir =
        resolve_sessions_dir(state.sessions_dir.clone()).map_err(ApiError::internal)?;
    let session_id = session.session_id.clone();
    let message_id_exists = run_blocking({
        let sessions_dir = sessions_dir.clone();
        let message_id = message_id.clone();
        move || {
            store::watch_web_queued_prompt_message_id_exists_from(
                &sessions_dir,
                &session_id,
                &message_id,
            )
        }
    })
    .await?;
    if message_id_exists {
        return Err(ApiError::conflict("message ID has already been used"));
    }
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
    let resume_pending = state.resume_pending.clone();
    let is_paused = session.status == SessionStatus::Paused;
    let pending_session_id = session.session_id.clone();
    let mut pending_attempt = None;
    let queued_prompt_id = send_queued_prompt_with_before_delivery(
        QueuedPromptDelivery {
            runtime: state.runtime,
            sessions_dir,
            session,
            marker,
            message_id: Some(message_id),
            action_event_kind: None,
            queued_content,
            message_content: content,
            queued_attachments,
            attachment_paths,
        },
        || async {
            if is_paused {
                pending_attempt = Some(
                    resume_pending
                        .mark_pending(pending_session_id.clone(), session_start_generation)
                        .await,
                );
            }
        },
    )
    .await;
    let queued_prompt_id = match queued_prompt_id {
        Ok(queued_prompt_id) => queued_prompt_id,
        Err(error) => {
            if let Some(attempt_id) = pending_attempt {
                resume_pending
                    .mark_stuck(&pending_session_id, attempt_id)
                    .await;
            }
            return Err(error);
        }
    };
    Ok(Json(MessageSuccess {
        ok: true,
        queued_prompt_id,
    }))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MessageSuccess {
    ok: bool,
    queued_prompt_id: String,
}

pub(super) struct QueuedPromptDelivery {
    pub(super) runtime: Arc<dyn super::ActionRuntime>,
    pub(super) sessions_dir: PathBuf,
    pub(super) session: Session,
    pub(super) marker: Option<String>,
    pub(super) message_id: Option<String>,
    pub(super) action_event_kind: Option<store::WatchWebActionEventKind>,
    pub(super) queued_content: String,
    pub(super) message_content: String,
    pub(super) queued_attachments: Vec<store::WatchWebQueuedAttachment>,
    pub(super) attachment_paths: Vec<PathBuf>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum QueueStoreFailurePolicy {
    RejectDelivery,
    DeliverWithoutQueue,
}

pub(super) async fn send_queued_prompt(delivery: QueuedPromptDelivery) -> Result<String, ApiError> {
    send_queued_prompt_with_before_delivery(delivery, || async {}).await
}

pub(super) async fn send_queued_prompt_with_before_delivery<F, Fut>(
    delivery: QueuedPromptDelivery,
    before_delivery: F,
) -> Result<String, ApiError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()>,
{
    send_queued_prompt_with_policy_and_before_delivery(
        delivery,
        QueueStoreFailurePolicy::RejectDelivery,
        before_delivery,
    )
    .await?
    .ok_or_else(|| ApiError::internal("queued prompt ID is missing after delivery"))
}

pub(super) async fn send_queued_prompt_best_effort(
    delivery: QueuedPromptDelivery,
) -> Result<Option<String>, ApiError> {
    send_queued_prompt_with_policy_and_before_delivery(
        delivery,
        QueueStoreFailurePolicy::DeliverWithoutQueue,
        || async {},
    )
    .await
}

async fn send_queued_prompt_with_policy_and_before_delivery<F, Fut>(
    delivery: QueuedPromptDelivery,
    queue_store_failure_policy: QueueStoreFailurePolicy,
    before_delivery: F,
) -> Result<Option<String>, ApiError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()>,
{
    let QueuedPromptDelivery {
        runtime,
        sessions_dir,
        session,
        marker,
        message_id,
        action_event_kind,
        queued_content,
        message_content,
        queued_attachments,
        attachment_paths,
    } = delivery;
    let message_id = message_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
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
                action_event_kind,
                sent_at,
            )
        }
    })
    .await;
    let queued_prompt_id = match queued_prompt_id {
        Ok(id) => id,
        Err(error) => {
            if queue_store_failure_policy == QueueStoreFailurePolicy::RejectDelivery {
                remove_attachments(attachment_paths).await;
                return Err(error);
            }
            tracing::warn!(
                session_id = %session.session_id,
                error = %error.message,
                "failed to save sent watch-web queued prompt; delivering without queue metadata"
            );
            before_delivery().await;
            let content = watch_web_message_content(&session, marker.as_deref(), &message_content);
            let send_session = session.clone();
            if let Err(delivery_error) =
                run_blocking(move || runtime.send_message(&send_session, &content)).await
            {
                remove_attachments(attachment_paths).await;
                return Err(delivery_error);
            }
            return Ok(None);
        }
    };

    before_delivery().await;
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
    let session_id = session.session_id.clone();
    if let Err(error) = run_blocking(move || {
        store::update_session_in(&sessions_dir, &session_id, |session| {
            if session
                .last_prompted_at
                .is_some_and(|last_prompted_at| last_prompted_at > sent_at)
            {
                return false;
            }
            session.last_prompted_at = Some(sent_at);
            true
        })
    })
    .await
    {
        tracing::warn!(
            session_id = %session.session_id,
            error = %error.message,
            "failed to save the prompt timestamp after delivering a watch-web message"
        );
    }
    Ok(Some(message_id))
}

pub(super) async fn remove_attachments(paths: Vec<PathBuf>) {
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

pub(super) struct ActionEventMessage<'a> {
    pub(super) marker: Option<&'a str>,
    pub(super) message_content: &'a str,
    pub(super) event_text: &'a str,
    pub(super) attachments: &'a [store::WatchWebQueuedAttachment],
    pub(super) action_event_kind: store::WatchWebActionEventKind,
}

pub(super) fn send_action_event_message(
    sessions_dir: &Path,
    session: &Session,
    message: ActionEventMessage<'_>,
    deliver: impl FnOnce(&Session, &str) -> Result<()>,
) -> Result<()> {
    let message_id = action_event::new_action_message_id();
    let queued_prompt_id = store::insert_watch_web_queued_prompt_from(
        sessions_dir,
        &session.session_id,
        &message_id,
        message.event_text,
        message.attachments,
        Some(message.action_event_kind),
        Utc::now(),
    )?;
    let content = watch_web_message_content(session, message.marker, message.message_content);
    let content = queued_prompt::add_id_marker(&content, &message_id);
    if let Err(error) = deliver(session, &content) {
        if let Err(cleanup_error) = store::delete_watch_web_queued_prompts_from(
            sessions_dir,
            &session.session_id,
            &[queued_prompt_id],
        ) {
            tracing::warn!(
                session_id = %session.session_id,
                error = %cleanup_error,
                "failed to remove unsent watch-web action event"
            );
            if let Err(resolve_error) = store::mark_watch_web_queued_prompts_resolved_from(
                sessions_dir,
                &session.session_id,
                &[queued_prompt_id],
            ) {
                tracing::warn!(
                    session_id = %session.session_id,
                    error = %resolve_error,
                    "failed to resolve unsent watch-web action event"
                );
            }
        }
        return Err(error);
    }
    Ok(())
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
    use chrono::TimeZone;
    use rstest::{fixture, rstest};
    use serde_json::json;
    use tempfile::tempdir;

    use super::{
        ActionEventMessage, is_valid_message_id, send_action_event_message, send_via_peer_notify,
    };
    use crate::commands::agent::peer::notify;
    use crate::commands::agent::store::{self, WatchWebActionEventKind};
    use crate::commands::agent::types::{Engine, Session};

    #[fixture]
    fn sessions_dir() -> tempfile::TempDir {
        tempdir().unwrap()
    }

    #[fixture]
    fn codex_session() -> Session {
        serde_json::from_value(json!({
            "session_id": "fixture-session",
            "engine": "codex",
            "cwd": "/fixture/worktree",
            "transcript_path": null,
            "tmux_info": null,
            "status": "running",
            "created_at": "2026-03-14T09:26:53Z",
            "updated_at": "2026-03-14T09:26:53Z",
            "last_message": null,
        }))
        .unwrap()
    }

    #[rstest]
    #[case::canonical_uuid("11111111-1111-4111-8111-111111111111", true)]
    #[case::malformed("not-a-uuid", false)]
    #[case::noncanonical_case("AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA", false)]
    fn validates_message_ids(#[case] message_id: &str, #[case] expected: bool) {
        assert_eq!(is_valid_message_id(message_id), expected);
    }

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

    #[rstest]
    #[case::claude(Engine::Claude)]
    #[case::codex(Engine::Codex)]
    fn action_delivery_records_its_kind_with_the_transcript_message_id(
        sessions_dir: tempfile::TempDir,
        codex_session: Session,
        #[case] engine: Engine,
    ) {
        let mut session = codex_session;
        session.engine = engine;
        let message_content = "<armyknife-todo-completed>The user marked the to-do as done: Review the fixture.</armyknife-todo-completed>";
        let mut delivered_message = None;

        send_action_event_message(
            sessions_dir.path(),
            &session,
            ActionEventMessage {
                marker: None,
                message_content,
                event_text: "The user marked the to-do as done: Review the fixture.",
                attachments: &[],
                action_event_kind: WatchWebActionEventKind::TodoCompleted,
            },
            |session, content| {
                delivered_message = Some((session.session_id.clone(), content.to_owned()));
                Ok(())
            },
        )
        .unwrap();

        let mut prompt =
            store::watch_web_queued_prompts_from(sessions_dir.path(), &session.session_id)
                .unwrap()
                .pop()
                .unwrap();
        let message_id = prompt.message_id.clone().unwrap();
        prompt.message_id = Some("<message-id>".to_owned());
        prompt.sent_at = chrono::Utc
            .with_ymd_and_hms(2026, 3, 14, 9, 26, 53)
            .unwrap();
        let delivered_message = delivered_message.unwrap();

        assert_eq!(
            (
                prompt,
                delivered_message.0,
                delivered_message.1.replace(&message_id, "<message-id>"),
            ),
            (
                store::WatchWebQueuedPrompt {
                    id: 1,
                    message_id: Some("<message-id>".to_owned()),
                    content: "The user marked the to-do as done: Review the fixture.".to_owned(),
                    action_event_kind: Some(WatchWebActionEventKind::TodoCompleted),
                    attachments: Vec::new(),
                    resolved: false,
                    sent_at: chrono::Utc
                        .with_ymd_and_hms(2026, 3, 14, 9, 26, 53)
                        .unwrap(),
                },
                "fixture-session".to_owned(),
                "<armyknife-todo-completed>The user marked the to-do as done: Review the fixture.</armyknife-todo-completed>\n<!-- armyknife-queued-prompt-id:<message-id> -->".to_owned(),
            ),
        );
    }

    #[rstest]
    fn codex_action_delivery_removes_its_record_when_delivery_fails(
        sessions_dir: tempfile::TempDir,
        codex_session: Session,
    ) {
        let message_content = "<armyknife-todo-rejected>The user rejected the to-do: Review the fixture.</armyknife-todo-rejected>";
        let error = send_action_event_message(
            sessions_dir.path(),
            &codex_session,
            ActionEventMessage {
                marker: None,
                message_content,
                event_text: "The user rejected the to-do: Review the fixture.",
                attachments: &[],
                action_event_kind: WatchWebActionEventKind::TodoRejected,
            },
            |_, _| Err(anyhow::anyhow!("delivery failed")),
        )
        .unwrap_err();
        let prompts =
            store::watch_web_queued_prompts_from(sessions_dir.path(), &codex_session.session_id)
                .unwrap();

        assert_eq!(
            (error.to_string(), prompts),
            (
                "delivery failed".to_owned(),
                Vec::<store::WatchWebQueuedPrompt>::new()
            ),
        );
    }
}
