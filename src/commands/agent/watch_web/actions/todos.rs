use axum::Json;
use axum::extract::{Path as RoutePath, State};
use serde::Deserialize;

use super::super::super::{store, types::SessionStatus};
use super::{ActionState, ApiError, Success, load_session, resolve_sessions_dir, run_blocking};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CompleteRequest {
    comment: Option<String>,
}

pub(super) async fn complete_todo(
    State(state): State<ActionState>,
    RoutePath((id, seq)): RoutePath<(String, i64)>,
    Json(request): Json<CompleteRequest>,
) -> Result<Json<Success>, ApiError> {
    let todo_lock = state.todo_completion_locks.get_or_create(&id, seq).await;
    let _todo_guard = todo_lock.lock().await;
    let session = load_session(state.clone(), id.clone()).await?;
    let configured_sessions_dir = state.sessions_dir.clone();
    let sessions_dir =
        resolve_sessions_dir(configured_sessions_dir.clone()).map_err(ApiError::internal)?;
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
        .ok_or_else(|| ApiError::conflict("todo is not awaiting completion"))?;
    if todo.completed_at.is_some() {
        return Err(ApiError::conflict("todo is not awaiting completion"));
    }
    let comment = request
        .comment
        .map(|comment| comment.trim().to_string())
        .filter(|comment| !comment.is_empty());
    if session.status != SessionStatus::Ended {
        let content = todo_completion_message(&todo.title, comment.as_deref());
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
    let completed = run_blocking(move || match configured_sessions_dir {
        Some(sessions_dir) => store::complete_agent_todo_from(
            &sessions_dir,
            &session_id,
            seq,
            comment.as_deref(),
        ),
        None => store::complete_agent_todo(&session_id, seq, comment.as_deref()),
    })
    .await
    .map_err(|error| {
        tracing::error!(error = %error.message, "failed to save sent watch-web todo completion");
        ApiError::operation(
            "todo completion was sent but could not be saved; check the conversation before retrying",
        )
    })?;
    if !completed {
        return Err(ApiError::conflict(
            "todo completion was sent but the todo is no longer available",
        ));
    }
    Ok(Json(Success::new()))
}

fn todo_completion_message(title: &str, comment: Option<&str>) -> String {
    let mut content =
        format!("<armyknife-todo-completed>The user marked the to-do as done: {title}.");
    if let Some(comment) = comment {
        content.push('\n');
        content.push('\n');
        content.push_str(comment);
    }
    content.push_str("</armyknife-todo-completed>");
    content
}
