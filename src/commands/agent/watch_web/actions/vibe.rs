use axum::Json;
use axum::extract::{Path as RoutePath, State};
use serde::Deserialize;
use serde_json::{Value, json};

use super::super::messages::action_event::{
    VIBE_MODE_OFF_NOTICE, VIBE_MODE_OFF_TAG, VIBE_MODE_ON_NOTICE, VIBE_MODE_ON_TAG,
    new_vibe_mode_message_id,
};
use crate::commands::agent::types::{Engine, SessionStatus};
use crate::commands::agent::{store, vibe};

use super::messages::{QueuedPromptDelivery, lock_session_action, send_queued_prompt};
use super::{ActionState, ApiError, load_session, resolve_sessions_dir, run_blocking};

#[derive(Deserialize)]
pub(super) struct VibeRequest {
    enabled: bool,
}

pub(super) async fn set_vibe(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
    Json(request): Json<VibeRequest>,
) -> Result<Json<Value>, ApiError> {
    let _session_action_guard = lock_session_action(&id).await;
    let session = load_session(state.clone(), id).await?;
    if session.status == SessionStatus::Ended {
        return Err(ApiError::conflict("session has ended"));
    }
    if session.sweep_signaled {
        return Err(ApiError::conflict("session is being stopped"));
    }

    let vibe_config = (state.vibe_config)();
    let repo_identity_resolver = state.repo_identity.clone();
    let cwd = session.cwd.clone();
    let repo_identity = run_blocking(move || Ok(repo_identity_resolver(&cwd))).await?;
    if request.enabled {
        let Some((owner, repo)) = repo_identity.as_ref() else {
            return Err(ApiError::conflict(
                "vibe mode is disabled because the session repository could not be identified",
            ));
        };
        vibe::ensure_repo_vibe_access(&vibe_config, owner, repo)
            .map_err(|error| ApiError::conflict(error.to_string()))?;
    }

    let sessions_dir = resolve_sessions_dir(state.sessions_dir).map_err(ApiError::internal)?;
    let resulting_vibe_mode = if request.enabled {
        vibe::VibeState::Enabled
    } else {
        let sessions_dir = sessions_dir.clone();
        let mut session_after_off = session.clone();
        session_after_off.vibe_enabled = false;
        let repo_identity = repo_identity.clone();
        run_blocking(move || {
            vibe::state_for_with_ancestor_loader(
                &session_after_off,
                repo_identity
                    .as_ref()
                    .map(|(owner, repo)| (owner.as_str(), repo.as_str())),
                &vibe_config,
                |ancestor_session_id| store::load_session_from(&sessions_dir, ancestor_session_id),
            )
        })
        .await?
    };
    // Delivery metadata must be available before persisting the requested setting.
    let marker = if session.engine == Engine::Claude {
        let sessions_dir = sessions_dir.clone();
        let session_id = session.session_id.clone();
        Some(
            run_blocking(move || {
                store::ensure_watch_web_user_marker_from(&sessions_dir, &session_id)
            })
            .await?,
        )
    } else {
        None
    };
    let restore_session_id = session.session_id.clone();
    let session_id = session.session_id.clone();
    let enabled = request.enabled;
    let change = run_blocking({
        let sessions_dir = sessions_dir.clone();
        move || vibe::set_in(&sessions_dir, &session_id, enabled)
    })
    .await?;

    let (event_tag, message) = if request.enabled {
        (VIBE_MODE_ON_TAG, VIBE_MODE_ON_NOTICE)
    } else {
        (VIBE_MODE_OFF_TAG, VIBE_MODE_OFF_NOTICE)
    };
    let event_message = format!("<{event_tag}>{message}</{event_tag}>");
    let delivery = send_queued_prompt(QueuedPromptDelivery {
        runtime: state.runtime,
        sessions_dir: sessions_dir.clone(),
        session,
        marker,
        message_id: Some(new_vibe_mode_message_id()),
        queued_content: event_message.clone(),
        message_content: event_message,
        queued_attachments: Vec::<store::WatchWebQueuedAttachment>::new(),
        attachment_paths: Vec::new(),
    })
    .await;
    let queued_prompt_id = match delivery {
        Ok(queued_prompt_id) => queued_prompt_id,
        Err(error) => {
            let session_id_for_log = restore_session_id.clone();
            let restore_result = run_blocking({
                let sessions_dir = sessions_dir.clone();
                let session_id = restore_session_id;
                move || vibe::restore_in_if_unchanged(&sessions_dir, &session_id, change)
            })
            .await;
            if let Err(restore_error) = restore_result {
                tracing::warn!(
                    session_id = %session_id_for_log,
                    error = %restore_error.message,
                    "failed to restore vibe setting after prompt delivery failed"
                );
            }
            return Err(error);
        }
    };

    Ok(Json(json!({
        "ok": true,
        "queuedPromptId": queued_prompt_id,
        "vibeMode": resulting_vibe_mode,
    })))
}
