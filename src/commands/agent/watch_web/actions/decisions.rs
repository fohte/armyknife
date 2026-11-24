use axum::Json;
use axum::extract::{Path as RoutePath, State};
use chrono::Utc;
use serde::Deserialize;

use super::super::super::{
    store,
    types::{DisplayStatus, Engine, Session, SessionStatus},
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
    let updated = run_blocking(move || {
        let exists = store::agent_decisions_from(&sessions_dir, &id)?
            .iter()
            .any(|decision| decision.seq == seq);
        if exists {
            store::set_agent_decision_review_from(&sessions_dir, &id, seq, "ok")?;
        }
        Ok(exists)
    })
    .await?;
    if !updated {
        return Err(ApiError::decision_not_found());
    }
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
    let has_running_background_task = session.display_status() == DisplayStatus::Background;
    let sessions_dir = resolve_sessions_dir(state.sessions_dir).map_err(ApiError::internal)?;
    let runtime = state.runtime;
    enum ChangeOutcome {
        Sent,
        DecisionNotFound,
        ParentNotFound,
        ParentArchived,
    }

    let outcome = run_blocking(move || {
        let Some(decision) = store::agent_decisions_from(&sessions_dir, &id)?
            .into_iter()
            .find(|decision| decision.seq == seq)
        else {
            return Ok(ChangeOutcome::DecisionNotFound);
        };

        let web_state = store::watch_web_session_states_from(&sessions_dir)?
            .into_iter()
            .find(|state| state.session_id == session.session_id);
        let is_done = session.status == SessionStatus::Ended
            || decision_category_is_done(
                &session,
                &config::load_config_or_default().agent,
                has_running_background_task,
                web_state.as_ref(),
                Utc::now(),
            );
        let parent_id = is_done
            .then(|| session.ancestor_session_ids.last())
            .flatten();
        let target = if let Some(parent_id) = parent_id {
            let Some(parent) = store::load_session_from(&sessions_dir, parent_id)? else {
                return Ok(ChangeOutcome::ParentNotFound);
            };
            if parent.archived_at.is_some() {
                return Ok(ChangeOutcome::ParentArchived);
            }
            parent
        } else {
            session.clone()
        };
        let body = change_message_body(&session, &target, &decision.title, &request.comment);
        let content = format!(
            "<{tag}>{body}</{tag}>",
            tag = super::super::messages::action_event::DECISION_CHANGE_TAG,
        );
        let marker = (target.engine == Engine::Claude)
            .then(|| store::ensure_watch_web_user_marker_from(&sessions_dir, &target.session_id))
            .transpose()?;
        let event_text = super::super::messages::action_event::event_text_for_kind(
            store::WatchWebActionEventKind::DecisionChange,
            &body,
        );
        super::messages::send_action_event_message(
            &sessions_dir,
            &target,
            super::messages::ActionEventMessage {
                marker: marker.as_deref(),
                message_content: &content,
                event_text: &event_text,
                attachments: &[],
                action_event_kind: store::WatchWebActionEventKind::DecisionChange,
            },
            |session, content| runtime.send_message(session, content),
        )?;
        store::set_agent_decision_review_from(&sessions_dir, &id, seq, "change")?;
        Ok(ChangeOutcome::Sent)
    })
    .await?;
    match outcome {
        ChangeOutcome::Sent => {}
        ChangeOutcome::DecisionNotFound => return Err(ApiError::decision_not_found()),
        ChangeOutcome::ParentNotFound => {
            return Err(ApiError::conflict("parent session not found"));
        }
        ChangeOutcome::ParentArchived => {
            return Err(ApiError::conflict("parent session is archived"));
        }
    }
    Ok(Json(Success::new()))
}

fn change_message_body(session: &Session, target: &Session, title: &str, comment: &str) -> String {
    let session_id = &session.session_id;
    let message = if session.session_id != target.session_id {
        indoc::formatdoc! {"
            Decision from session {session_id}: {title}

            Requested change: {comment}
        "}
    } else {
        indoc::formatdoc! {"
            Requested change to decision: {title}

            {comment}
        "}
    };
    message.trim_end().to_owned()
}
