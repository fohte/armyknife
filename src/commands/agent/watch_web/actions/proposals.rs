use anyhow::Context;
use axum::Json;
use axum::extract::{Path as RoutePath, State};
use serde::Deserialize;

use super::super::super::{
    proposal::{ProposalAnswer, ProposalBody},
    store,
    types::{Engine, SessionStatus},
};
use super::{
    ActionState, ApiError, Success, load_session, messages::watch_web_message_content,
    resolve_sessions_dir, run_blocking,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AnswerRequest {
    choice: serde_json::Value,
    comment: Option<String>,
}

pub(super) async fn answer_proposal(
    State(state): State<ActionState>,
    RoutePath((id, seq)): RoutePath<(String, i64)>,
    Json(request): Json<AnswerRequest>,
) -> Result<Json<Success>, ApiError> {
    let proposal_answer_lock = state.proposal_answer_lock(&id, seq).await;
    let _answer_guard = proposal_answer_lock.lock().await;
    let session = load_session(state.clone(), id.clone()).await?;
    if session.status == SessionStatus::Ended {
        return Err(ApiError::conflict("session has ended"));
    }
    let configured_sessions_dir = state.sessions_dir.clone();
    let sessions_dir =
        resolve_sessions_dir(configured_sessions_dir.clone()).map_err(ApiError::internal)?;
    let proposals = run_blocking({
        let session_id = session.session_id.clone();
        move || match configured_sessions_dir {
            Some(sessions_dir) => store::agent_proposals_from(&sessions_dir, &session_id),
            None => store::agent_proposals(&session_id),
        }
    })
    .await?;
    let proposal = proposals
        .iter()
        .find(|proposal| proposal.seq == seq)
        .ok_or_else(|| ApiError::conflict("proposal is not awaiting an answer"))?;
    if proposal.resolved_at.is_some() {
        return Err(ApiError::conflict("proposal is not awaiting an answer"));
    }
    let body: ProposalBody = serde_json::from_str(&proposal.body)
        .context("parsing stored proposal body")
        .map_err(ApiError::internal)?;
    let choice = if request.choice.is_null() {
        None
    } else {
        let choice = request
            .choice
            .as_u64()
            .and_then(|choice| usize::try_from(choice).ok())
            .ok_or_else(|| ApiError::bad_request("choice is outside the proposal options"))?;
        if choice >= body.options.len() {
            return Err(ApiError::bad_request(
                "choice is outside the proposal options",
            ));
        }
        Some(choice)
    };
    let choice_label = match choice {
        Some(choice) => {
            let choice = u8::try_from(choice)
                .map_err(|_| ApiError::bad_request("choice is outside the proposal options"))?;
            Some((
                char::from(b'A' + choice),
                body.options[usize::from(choice)].label.as_str(),
            ))
        }
        None => None,
    };
    let content =
        proposal_answer_message(seq, &body.title, choice_label, request.comment.as_deref());
    let marker = if session.engine == Engine::Claude {
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
    let content = watch_web_message_content(&session, marker.as_deref(), &content);
    let runtime = state.runtime;
    let send_session = session.clone();
    run_blocking(move || runtime.send_message(&send_session, &content)).await?;

    let answer = serde_json::to_string(&ProposalAnswer {
        choice,
        comment: request.comment,
    })
    .map_err(ApiError::internal)?;
    let session_id = session.session_id;
    let configured_sessions_dir = state.sessions_dir;
    let updated = run_blocking(move || match configured_sessions_dir {
        Some(sessions_dir) => {
            store::write_agent_proposal_answer_from(&sessions_dir, &session_id, seq, &answer)
        }
        None => store::write_agent_proposal_answer(&session_id, seq, &answer),
    })
    .await?;
    if !updated {
        return Err(ApiError::conflict("proposal is no longer available"));
    }
    Ok(Json(Success::new()))
}

fn proposal_answer_message(
    seq: i64,
    title: &str,
    choice_label: Option<(char, &str)>,
    comment: Option<&str>,
) -> String {
    let mut content = match choice_label {
        Some((choice, label)) => format!("Proposal {seq} \"{title}\": chose {choice} ({label})."),
        None => {
            format!("Proposal {seq} \"{title}\": none of the options fit. Propose another option.")
        }
    };
    if let Some(comment) = comment {
        content.push('\n');
        content.push('\n');
        content.push_str(comment);
    }
    content
}
