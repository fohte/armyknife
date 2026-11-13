use axum::Json;
use axum::extract::{Path as RoutePath, State};
use chrono::Utc;
use serde_json::json;

use super::super::super::{
    store,
    types::{Engine, SessionStatus},
};
use super::{
    ActionState, ApiError, Success, load_session, messages::watch_web_message_content,
    resolve_sessions_dir, run_blocking,
};

pub(super) async fn retry_permission_denied(
    State(state): State<ActionState>,
    RoutePath((id, seq)): RoutePath<(String, i64)>,
) -> Result<Json<Success>, ApiError> {
    let retry_lock = state
        .permission_denied_retry_locks
        .get_or_create(&id, seq)
        .await;
    let _retry_guard = retry_lock.lock().await;
    let session = load_session(state.clone(), id.clone()).await?;
    if session.engine != Engine::Claude {
        return Err(ApiError::conflict(
            "permission retry is unavailable for this session",
        ));
    }
    if session.status == SessionStatus::Ended {
        return Err(ApiError::conflict("session has ended"));
    }

    let sessions_dir =
        resolve_sessions_dir(state.sessions_dir.clone()).map_err(ApiError::internal)?;
    let session_id = session.session_id.clone();
    let denied = run_blocking({
        let session_id = session_id.clone();
        let sessions_dir = sessions_dir.clone();
        move || store::permission_denied_operations_from(&sessions_dir, &session_id)
    })
    .await?
    .into_iter()
    .find(|operation| operation.seq == seq)
    .ok_or_else(|| ApiError::conflict("denied operation is no longer available"))?;
    if denied.resolved_at.is_some() {
        return Err(ApiError::conflict("denied operation was already retried"));
    }
    if !denied.has_classifier_verdict() {
        return Err(ApiError::conflict(
            "this denial has no classifier verdict and cannot be approved",
        ));
    }

    let operation = serde_json::to_string(&json!({
        "tool_name": denied.tool_name,
        "tool_input": denied.tool_input,
        "tool_use_id": denied.tool_use_id,
    }))
    .map_err(ApiError::internal)?;
    let queue_content = indoc::formatdoc! {"
        I approve retrying this exact blocked operation once.
        Please perform this tool call one time only:
        {operation}
        This approval applies only to the operation shown above.
    "};
    let marker = run_blocking({
        let sessions_dir = sessions_dir.clone();
        let session_id = session_id.clone();
        move || store::ensure_watch_web_user_marker_from(&sessions_dir, &session_id)
    })
    .await?;
    let content = watch_web_message_content(&session, Some(&marker), &queue_content);
    let runtime = state.runtime;
    let send_session = session.clone();
    run_blocking(move || runtime.send_message(&send_session, &content)).await?;

    let saved = run_blocking({
        let sessions_dir = sessions_dir.clone();
        let session_id = session_id.clone();
        move || store::resolve_permission_denied_from(&sessions_dir, &session_id, seq)
    })
    .await
    .map_err(|error| {
        tracing::error!(error = %error.message, "failed to save sent permission-denied retry");
        ApiError::operation(
            "retry instruction was sent but the card could not be marked complete; check the conversation before retrying",
        )
    })?;
    if !saved {
        return Err(ApiError::conflict(
            "retry instruction was sent but the denied operation is no longer available",
        ));
    }
    if let Err(error) = store::insert_watch_web_queued_prompt_from(
        &sessions_dir,
        &session_id,
        &queue_content,
        Utc::now(),
    ) {
        tracing::warn!(
            session_id = %session_id,
            %error,
            "failed to save sent permission-denied retry message"
        );
    }
    Ok(Json(Success::new()))
}
