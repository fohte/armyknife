use axum::Json;
use axum::extract::{Path as RoutePath, State};
use chrono::{Duration, Utc};
use serde_json::json;
use std::path::PathBuf;
use tokio::sync::OwnedMutexGuard;

use super::super::super::store::PermissionDeniedResolution;
use super::super::super::{
    store,
    types::{Engine, Session, SessionStatus},
};
use super::{ActionState, ApiError, Success, load_session, resolve_sessions_dir, run_blocking};

struct UnresolvedPermissionDenial {
    _guard: OwnedMutexGuard<()>,
    session: Session,
    sessions_dir: PathBuf,
    operation: store::PermissionDeniedOperation,
}

const PERMISSION_RETRY_APPROVAL_TTL: Duration = Duration::minutes(5);

pub(super) async fn retry_permission_denied(
    State(state): State<ActionState>,
    RoutePath((id, seq)): RoutePath<(String, i64)>,
) -> Result<Json<Success>, ApiError> {
    let UnresolvedPermissionDenial {
        _guard: _denial_guard,
        session,
        sessions_dir,
        operation: denied,
    } = load_unresolved_permission_denial(
        &state,
        &id,
        seq,
        "denied operation was already retried",
        |session| {
            if session.engine != Engine::Claude {
                return Err(ApiError::conflict(
                    "permission retry is unavailable for this session",
                ));
            }
            if session.status == SessionStatus::Ended {
                return Err(ApiError::conflict("session has ended"));
            }
            Ok(())
        },
    )
    .await?;
    let session_id = session.session_id.clone();
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
    let approval_id = run_blocking({
        let sessions_dir = sessions_dir.clone();
        let session_id = session_id.clone();
        let tool_name = denied.tool_name.clone();
        let tool_input = denied.tool_input.clone();
        let expires_at = (Utc::now() + PERMISSION_RETRY_APPROVAL_TTL).timestamp();
        move || {
            store::insert_permission_retry_approval_from(
                &sessions_dir,
                &session_id,
                &tool_name,
                &tool_input,
                expires_at,
            )
        }
    })
    .await?;
    let delivery = super::messages::send_queued_prompt(super::messages::QueuedPromptDelivery {
        runtime: state.runtime,
        sessions_dir: sessions_dir.clone(),
        session,
        marker: Some(marker),
        queued_content: queue_content.clone(),
        message_content: queue_content,
        queued_attachments: Vec::new(),
        attachment_paths: Vec::new(),
    })
    .await;
    if let Err(error) = delivery {
        let revoke_result = run_blocking({
            let sessions_dir = sessions_dir.clone();
            move || store::revoke_permission_retry_approval_from(&sessions_dir, approval_id)
        })
        .await;
        if let Err(revoke_error) = revoke_result {
            tracing::error!(
                error = %revoke_error.message,
                "failed to revoke unsent permission-retry approval",
            );
        }
        return Err(error);
    }

    let saved = run_blocking({
        let sessions_dir = sessions_dir.clone();
        let session_id = session_id.clone();
        move || {
            store::resolve_permission_denied_from(
                &sessions_dir,
                &session_id,
                seq,
                PermissionDeniedResolution::Approved,
            )
        }
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
    Ok(Json(Success::new()))
}

pub(super) async fn dismiss_permission_denied(
    State(state): State<ActionState>,
    RoutePath((id, seq)): RoutePath<(String, i64)>,
) -> Result<Json<Success>, ApiError> {
    let UnresolvedPermissionDenial {
        _guard: _denial_guard,
        session,
        sessions_dir,
        operation: _denial,
    } = load_unresolved_permission_denial(
        &state,
        &id,
        seq,
        "denied operation was already resolved",
        |_| Ok(()),
    )
    .await?;
    let session_id = session.session_id.clone();
    let saved = run_blocking(move || {
        store::resolve_permission_denied_from(
            &sessions_dir,
            &session_id,
            seq,
            PermissionDeniedResolution::Dismissed,
        )
    })
    .await?;
    if !saved {
        return Err(ApiError::conflict("denied operation was already resolved"));
    }
    Ok(Json(Success::new()))
}

async fn load_unresolved_permission_denial(
    state: &ActionState,
    id: &str,
    seq: i64,
    resolved_message: &'static str,
    validate_session: impl FnOnce(&Session) -> Result<(), ApiError>,
) -> Result<UnresolvedPermissionDenial, ApiError> {
    let denial_lock = state.permission_denied_locks.get_or_create(id, seq).await;
    let denial_guard = denial_lock.lock_owned().await;
    let session = load_session(state.clone(), id.to_owned()).await?;
    validate_session(&session)?;
    let sessions_dir =
        resolve_sessions_dir(state.sessions_dir.clone()).map_err(ApiError::internal)?;
    let session_id = session.session_id.clone();
    let operation = run_blocking({
        let sessions_dir = sessions_dir.clone();
        let session_id = session_id.clone();
        move || store::permission_denied_operations_from(&sessions_dir, &session_id)
    })
    .await?
    .into_iter()
    .find(|operation| operation.seq == seq)
    .ok_or_else(|| ApiError::conflict("denied operation is no longer available"))?;
    if operation.resolved_at.is_some() {
        return Err(ApiError::conflict(resolved_message));
    }

    Ok(UnresolvedPermissionDenial {
        _guard: denial_guard,
        session,
        sessions_dir,
        operation,
    })
}
