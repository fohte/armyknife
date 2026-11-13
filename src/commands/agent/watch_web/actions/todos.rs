use axum::Json;
use axum::extract::{Path as RoutePath, State};
use serde::Deserialize;

use super::super::super::{store, types::SessionStatus};
use super::{ActionState, ApiError, Success, load_session, resolve_sessions_dir, run_blocking};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResolveRequest {
    resolution: UserTodoResolution,
    comment: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum UserTodoResolution {
    Completed,
    Rejected,
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
    let resolution = match request.resolution {
        UserTodoResolution::Completed => store::TodoResolution::Completed,
        UserTodoResolution::Rejected => store::TodoResolution::Rejected,
    };
    let comment = request
        .comment
        .map(|comment| comment.trim().to_string())
        .filter(|comment| !comment.is_empty());
    if session.status != SessionStatus::Ended {
        let content = todo_resolution_message(request.resolution, &todo.title, comment.as_deref());
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
        let content =
            super::messages::watch_web_message_content(&session, marker.as_deref(), &content);
        let runtime = state.runtime;
        let send_session = session.clone();
        run_blocking(move || runtime.send_message(&send_session, &content)).await?;
    }

    let session_id = session.session_id;
    let configured_sessions_dir = state.sessions_dir;
    let resolved = run_blocking(move || match configured_sessions_dir {
        Some(sessions_dir) => store::resolve_agent_todo_from(
            &sessions_dir,
            &session_id,
            seq,
            resolution,
            comment.as_deref(),
        ),
        None => store::resolve_agent_todo(&session_id, seq, resolution, comment.as_deref()),
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
) -> String {
    let (tag, summary) = match resolution {
        UserTodoResolution::Completed => (
            "armyknife-todo-completed",
            format!("The user marked the to-do as done: {title}."),
        ),
        UserTodoResolution::Rejected => (
            "armyknife-todo-rejected",
            format!("The user rejected the to-do: {title}."),
        ),
    };
    let mut content = format!("<{tag}>{summary}");
    if let Some(comment) = comment {
        content.push('\n');
        content.push('\n');
        content.push_str(comment);
    }
    content.push_str(&format!("</{tag}>"));
    content
}
