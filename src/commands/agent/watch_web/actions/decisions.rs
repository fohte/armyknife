use axum::Json;
use axum::extract::{Path as RoutePath, State};
use chrono::Utc;
use serde::Deserialize;

use super::super::super::{
    store,
    types::{Engine, Session, SessionStatus},
};
use super::{ActionState, ApiError, Success, load_session, resolve_sessions_dir, run_blocking};
use crate::commands::agent::watch_web::sessions::decision_category_is_done;
use crate::shared::config;

pub(super) async fn mark_ok(
    State(state): State<ActionState>,
    RoutePath((id, seq)): RoutePath<(String, i64)>,
) -> Result<Json<Success>, ApiError> {
    let _session = load_session(state.clone(), id.clone()).await?;
    let sessions_dir = resolve_sessions_dir(state.sessions_dir).map_err(ApiError::internal)?;
    run_blocking(move || store::set_agent_decision_review_from(&sessions_dir, &id, seq, "ok"))
        .await?;
    Ok(Json(Success::new()))
}

#[derive(Deserialize)]
pub(super) struct ChangeRequest {
    comment: String,
}

pub(super) async fn request_change(
    State(state): State<ActionState>,
    RoutePath((id, seq)): RoutePath<(String, i64)>,
    Json(request): Json<ChangeRequest>,
) -> Result<Json<Success>, ApiError> {
    let session = load_session(state.clone(), id.clone()).await?;
    if request.comment.trim().is_empty() {
        return Err(ApiError::bad_request("comment must not be empty"));
    }
    let sessions_dir = resolve_sessions_dir(state.sessions_dir).map_err(ApiError::internal)?;
    let runtime = state.runtime;
    run_blocking(move || {
        let Some(decision) = store::agent_decisions_from(&sessions_dir, &id)?
            .into_iter()
            .find(|decision| decision.seq == seq)
        else {
            return Ok(false);
        };

        let web_state = store::watch_web_session_states_from(&sessions_dir)?
            .into_iter()
            .find(|state| state.session_id == session.session_id);
        let is_done = session.status == SessionStatus::Ended
            || decision_category_is_done(
                &session,
                &config::load_config_or_default().agent,
                web_state.as_ref(),
                Utc::now(),
            );
        let parent_id = is_done
            .then(|| session.ancestor_session_ids.last())
            .flatten();
        let target = if let Some(parent_id) = parent_id {
            store::load_session_from(&sessions_dir, parent_id)?
                .ok_or_else(|| anyhow::anyhow!("parent session not found: {parent_id}"))?
        } else {
            session.clone()
        };
        let content = change_message(&session, &target, &decision.title, &request.comment);
        let marker = (target.engine == Engine::Claude)
            .then(|| store::ensure_watch_web_user_marker_from(&sessions_dir, &target.session_id))
            .transpose()?;
        let content =
            super::messages::watch_web_message_content(&target, marker.as_deref(), &content);
        runtime.send_message(&target, &content)?;
        store::set_agent_decision_review_from(&sessions_dir, &id, seq, "change")?;
        Ok(true)
    })
    .await?
    .then_some(())
    .ok_or_else(ApiError::not_found)?;
    Ok(Json(Success::new()))
}

fn change_message(session: &Session, target: &Session, title: &str, comment: &str) -> String {
    let session_id = &session.session_id;
    if session.session_id != target.session_id {
        indoc::formatdoc! {"
            Decision from session {session_id}: {title}

            Requested change: {comment}
        "}
    } else {
        indoc::formatdoc! {"
            Requested change to decision: {title}

            {comment}
        "}
    }
}
