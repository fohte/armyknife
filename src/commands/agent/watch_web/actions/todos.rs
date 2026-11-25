use axum::Json;
use axum::extract::{Path as RoutePath, State};
use chrono::Utc;
use serde::Deserialize;

use super::super::super::{store, types::SessionStatus};
use super::super::attachments;
use super::request_fields::{deserialize_present, deserialize_present_nullable};
use super::{
    ActionState, ApiError, Success, load_session, messages::remove_attachments,
    resolve_sessions_dir, run_blocking,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResolveRequest {
    resolution: UserTodoResolution,
    comment: Option<String>,
    #[serde(
        rename = "expectedTitle",
        default,
        deserialize_with = "deserialize_present"
    )]
    expected_title: Option<String>,
    #[serde(
        rename = "expectedDetail",
        default,
        deserialize_with = "deserialize_present_nullable"
    )]
    expected_detail: Option<Option<String>>,
    #[serde(default)]
    attachments: Vec<attachments::Request>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FoldRequest {
    folded: bool,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum UserTodoResolution {
    Completed,
    Rejected,
}

pub(super) async fn fold_todo(
    State(state): State<ActionState>,
    RoutePath((id, seq)): RoutePath<(String, i64)>,
    Json(request): Json<FoldRequest>,
) -> Result<Json<Success>, ApiError> {
    let session = load_session(state.clone(), id).await?;
    let configured_sessions_dir = state.sessions_dir;
    let session_id = session.session_id;
    let found = run_blocking(move || match configured_sessions_dir {
        Some(sessions_dir) => {
            store::set_agent_todo_folded_from(&sessions_dir, &session_id, seq, request.folded)
        }
        None => store::set_agent_todo_folded(&session_id, seq, request.folded),
    })
    .await?;
    if !found {
        return Err(ApiError::conflict("todo is not pending"));
    }
    Ok(Json(Success::new()))
}

pub(super) async fn resolve_todo(
    State(state): State<ActionState>,
    RoutePath((id, seq)): RoutePath<(String, i64)>,
    Json(request): Json<ResolveRequest>,
) -> Result<Json<Success>, ApiError> {
    let session = load_session(state.clone(), id.clone()).await?;
    let configured_sessions_dir = state.sessions_dir.clone();
    let sessions_dir =
        resolve_sessions_dir(configured_sessions_dir.clone()).map_err(ApiError::internal)?;
    let session_id = session.session_id.clone();
    let locked_sessions_dir = sessions_dir.clone();
    let _todo_guard =
        run_blocking(move || store::lock_agent_todo_resolution(&locked_sessions_dir, &session_id))
            .await?;
    let todos = run_blocking({
        let session_id = session.session_id.clone();
        move || match configured_sessions_dir {
            Some(sessions_dir) => store::agent_todos_from(&sessions_dir, &session_id),
            None => store::agent_todos(&session_id),
        }
    })
    .await?;
    let todo = todos
        .iter()
        .find(|todo| todo.seq == seq)
        .ok_or_else(|| ApiError::conflict("todo is not pending"))?;
    if todo.completed_at.is_some() {
        return Err(ApiError::conflict("todo is not pending"));
    }
    if request
        .expected_title
        .as_ref()
        .is_some_and(|expected_title| &todo.title != expected_title)
        || request
            .expected_detail
            .as_ref()
            .is_some_and(|expected_detail| &todo.detail != expected_detail)
    {
        return Err(ApiError::conflict(
            "todo changed; reload it before resolving",
        ));
    }
    let attachments = attachments::validate(request.attachments, vec![])
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    if session.status == SessionStatus::Ended && !attachments.is_empty() {
        return Err(ApiError::conflict("session has ended"));
    }
    let resolution = match request.resolution {
        UserTodoResolution::Completed => store::TodoResolution::Completed,
        UserTodoResolution::Rejected => store::TodoResolution::Rejected,
    };
    let comment = request
        .comment
        .map(|comment| comment.trim().to_string())
        .filter(|comment| !comment.is_empty());
    let mut stored_comment = comment.clone();
    let completed_at = Utc::now();
    if session.status != SessionStatus::Ended {
        let (action_event_kind, body, content) =
            todo_resolution_message(request.resolution, &todo.title, comment.as_deref());
        let marker = if session.engine == super::super::super::types::Engine::Claude {
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
        stored_comment =
            attachments::append_image_previews_to_comment(stored_comment, &saved_attachments);
        let content = attachments::append_paths_to_message(content, &attachment_paths);
        let event_text =
            super::super::messages::action_event::event_text_for_kind(action_event_kind, &body);
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
        let runtime = state.runtime;
        let send_session = session.clone();
        let send_sessions_dir = sessions_dir.clone();
        if let Err(error) = run_blocking(move || {
            super::messages::send_action_event_message(
                &send_sessions_dir,
                &send_session,
                super::messages::ActionEventMessage {
                    marker: marker.as_deref(),
                    message_content: &content,
                    event_text: &event_text,
                    attachments: &queued_attachments,
                    action_event_kind,
                },
                |session, content| runtime.send_message(session, content),
            )
        })
        .await
        {
            remove_attachments(attachment_paths).await;
            return Err(error);
        }
    }

    let session_id = session.session_id;
    let configured_sessions_dir = state.sessions_dir;
    let resolved = run_blocking(move || match configured_sessions_dir {
        Some(sessions_dir) => store::resolve_agent_todo_from_at(
            &sessions_dir,
            &session_id,
            seq,
            resolution,
            stored_comment.as_deref(),
            completed_at,
        ),
        None => store::resolve_agent_todo_at(
            &session_id,
            seq,
            resolution,
            stored_comment.as_deref(),
            completed_at,
        ),
    })
    .await
    .map_err(|error| {
        tracing::error!(error = %error.message, "failed to save sent watch-web todo resolution");
        ApiError::operation(
            "todo resolution was sent but could not be saved; check the conversation before retrying",
        )
    })?;
    if !resolved {
        return Err(ApiError::conflict(
            "todo resolution was sent but the todo is no longer available",
        ));
    }
    Ok(Json(Success::new()))
}

fn todo_resolution_message(
    resolution: UserTodoResolution,
    title: &str,
    comment: Option<&str>,
) -> (store::WatchWebActionEventKind, String, String) {
    let (kind, tag, summary) = match resolution {
        UserTodoResolution::Completed => (
            store::WatchWebActionEventKind::TodoCompleted,
            super::super::messages::action_event::TODO_COMPLETED_TAG,
            format!("The user marked the to-do as done: {title}."),
        ),
        UserTodoResolution::Rejected => (
            store::WatchWebActionEventKind::TodoRejected,
            super::super::messages::action_event::TODO_REJECTED_TAG,
            format!("The user rejected the to-do: {title}."),
        ),
    };
    let mut body = summary;
    if let Some(comment) = comment {
        body.push('\n');
        body.push('\n');
        body.push_str(comment);
    }
    let content = format!("<{tag}>{body}</{tag}>");
    (kind, body, content)
}
