use axum::Json;
use axum::extract::{Path as RoutePath, State};
use serde::Deserialize;
use serde_json::{Value, json};

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

    let repo_identity = if request.enabled {
        let repo_identity = state.repo_identity.clone();
        let cwd = session.cwd.clone();
        run_blocking(move || Ok(repo_identity(&cwd))).await?
    } else {
        None
    };
    if request.enabled {
        let Some((owner, repo)) = repo_identity.as_ref() else {
            return Err(ApiError::conflict(
                "vibe mode is disabled because the session repository could not be identified",
            ));
        };
        vibe::ensure_repo_vibe_access(&state.vibe_config, owner, repo)
            .map_err(|error| ApiError::conflict(error.to_string()))?;
    }

    let sessions_dir = resolve_sessions_dir(state.sessions_dir).map_err(ApiError::internal)?;
    let session_id = session.session_id.clone();
    let enabled = request.enabled;
    run_blocking({
        let sessions_dir = sessions_dir.clone();
        move || vibe::set_in(&sessions_dir, &session_id, enabled)
    })
    .await?;

    let ancestors = run_blocking({
        let sessions_dir = sessions_dir.clone();
        let ancestor_ids = session.ancestor_session_ids.clone();
        move || {
            ancestor_ids
                .iter()
                .filter_map(|ancestor_id| {
                    match store::load_session_from(&sessions_dir, ancestor_id) {
                        Ok(Some(ancestor)) => Some(Ok(ancestor)),
                        Ok(None) => None,
                        Err(error) => Some(Err(error)),
                    }
                })
                .collect::<anyhow::Result<Vec<_>>>()
        }
    })
    .await?;
    let inherited_from = if request.enabled {
        None
    } else {
        vibe::inherited_vibe_ancestor(&session, &ancestors).map(str::to_string)
    };

    let message = match (request.enabled, inherited_from) {
        (true, _) => "watch-web からユーザーが vibe mode を on にしました。".to_string(),
        (false, Some(ancestor_session_id)) => format!(
            "watch-web からユーザーが vibe mode を off にしました。祖先セッション {ancestor_session_id} から引き継いでいるため、vibe mode は引き続き有効です。"
        ),
        (false, None) => "watch-web からユーザーが vibe mode を off にしました。".to_string(),
    };
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
    let queued_prompt_id = send_queued_prompt(QueuedPromptDelivery {
        runtime: state.runtime,
        sessions_dir,
        session,
        marker,
        queued_content: message.clone(),
        message_content: message,
        queued_attachments: Vec::<store::WatchWebQueuedAttachment>::new(),
        attachment_paths: Vec::new(),
    })
    .await?;

    Ok(Json(json!({
        "ok": true,
        "queuedPromptId": queued_prompt_id,
    })))
}
