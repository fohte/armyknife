use std::collections::BTreeMap;

use axum::Json;
use axum::extract::{Path as RoutePath, State};
use serde::Deserialize;

use super::super::super::store;
use super::super::super::types::{Engine, SessionStatus};
use super::{ActionState, ApiError, Success, load_session, resolve_sessions_dir, run_blocking};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AnswerRequest {
    request_id: Option<String>,
    answers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    skip: bool,
}

pub(super) async fn answer_question(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
    Json(request): Json<AnswerRequest>,
) -> Result<Json<Success>, ApiError> {
    let session = load_session(state.clone(), id).await?;
    if session.engine != Engine::Claude {
        return Err(ApiError::conflict(
            "session is not waiting for a question answer",
        ));
    }
    if session.status == SessionStatus::Ended {
        return Err(ApiError::conflict("session has ended"));
    }
    let request_id = request
        .request_id
        .filter(|request_id| !request_id.trim().is_empty())
        .ok_or_else(|| ApiError::bad_request("requestId is required"))?;
    let Some(agent_key) = session
        .pending_permission_request_ids
        .iter()
        .find(|(agent_key, pending_request_id)| {
            pending_request_id.as_str() == request_id
                && session
                    .pending_permission_agent_ids
                    .contains(agent_key.as_str())
        })
        .map(|(agent_key, _)| agent_key.clone())
    else {
        return Err(ApiError::conflict(
            "question request is no longer pending; answer it in the terminal",
        ));
    };
    let sessions_dir =
        resolve_sessions_dir(state.sessions_dir.clone()).map_err(ApiError::internal)?;
    let permission_data = run_blocking({
        let sessions_dir = sessions_dir.clone();
        let session_id = session.session_id.clone();
        let request_id = request_id.clone();
        move || store::permission_request_data(&sessions_dir, &session_id, &request_id)
    })
    .await?;
    let Some(permission_data) = permission_data
        .filter(|data| data.pending && data.tool_name.as_deref() == Some("AskUserQuestion"))
    else {
        return Err(ApiError::conflict(
            "question request is no longer pending; answer it in the terminal",
        ));
    };

    if request.skip {
        if request.answers.is_some() {
            return Err(ApiError::bad_request(
                "answers cannot be provided when skipping a question",
            ));
        }
        resolve_question_request(
            state,
            session.session_id,
            agent_key,
            request_id,
            store::PermissionRequestDecision::Deny {
                message: "The user skipped this question in watch-web.".to_string(),
                interrupt: false,
            },
            None,
        )
        .await?;
    } else {
        let answers = request
            .answers
            .ok_or_else(|| ApiError::bad_request("answers are required unless skipping"))?;
        validate_question_answers(&permission_data.tool_input, &answers)?;
        resolve_question_request(
            state,
            session.session_id,
            agent_key,
            request_id,
            store::PermissionRequestDecision::Allow,
            Some(serde_json::to_value(answers).map_err(ApiError::internal)?),
        )
        .await?;
    }
    Ok(Json(Success::new()))
}

fn validate_question_answers(
    tool_input: &serde_json::Value,
    answers: &BTreeMap<String, String>,
) -> Result<(), ApiError> {
    let questions = tool_input
        .get("questions")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| ApiError::bad_request("question request has no questions"))?;
    if questions.len() != answers.len() {
        return Err(ApiError::bad_request(
            "an answer is required for every question",
        ));
    }
    for question in questions {
        let question_text = question
            .get("question")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| ApiError::bad_request("question text is missing"))?;
        let answer = answers
            .get(question_text)
            .ok_or_else(|| ApiError::bad_request("an answer is required for every question"))?;
        if answer.trim().is_empty() {
            return Err(ApiError::bad_request("answers cannot be empty"));
        }
    }
    Ok(())
}

async fn resolve_question_request(
    state: ActionState,
    session_id: String,
    agent_key: String,
    request_id: String,
    decision: store::PermissionRequestDecision,
    answers: Option<serde_json::Value>,
) -> Result<(), ApiError> {
    let sessions_dir = resolve_sessions_dir(state.sessions_dir).map_err(ApiError::internal)?;
    let resolution = run_blocking(move || {
        store::resolve_permission_request_with_answers(
            &sessions_dir,
            &session_id,
            &agent_key,
            &request_id,
            &decision,
            answers.as_ref(),
        )
    })
    .await?;
    match resolution {
        store::PermissionRequestResolution::Resolved => Ok(()),
        store::PermissionRequestResolution::Missing
        | store::PermissionRequestResolution::Finished => Err(ApiError::conflict(
            "question request is no longer pending; answer it in the terminal",
        )),
    }
}
