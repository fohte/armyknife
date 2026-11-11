use axum::http::StatusCode;
use indoc::indoc;
use rstest::rstest;
use serde_json::json;
use tower::ServiceExt;

use super::{ActionContext, action_context, post, response_snapshot, session};
use crate::commands::agent::store;
use crate::commands::agent::types::Engine;

fn add_proposal(context: &ActionContext, session_id: &str) {
    store::add_agent_proposal_in(
        &context.temp_dir.path().join("sessions"),
        session_id,
        r#"{"title":"Pick a format","summary":null,"options":[{"label":"Format A","description":null,"pros":[],"cons":[],"recommended":false},{"label":"Format B","description":null,"pros":[],"cons":[],"recommended":false}]}"#,
    )
    .unwrap();
}

#[rstest]
#[tokio::test]
async fn chosen_option_is_notified_before_the_answer_is_stored(action_context: ActionContext) {
    action_context.save_session(&session("sample-session"));
    add_proposal(&action_context, "sample-session");

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/proposals/1/answer",
            json!({ "choice": 0, "comment": "Keep the simpler format." }),
        ))
        .await
        .unwrap();
    let proposals = store::agent_proposals_from(
        &action_context.temp_dir.path().join("sessions"),
        "sample-session",
    )
    .unwrap();

    assert_eq!(
        (
            response_snapshot(response, &action_context).await,
            proposals[0].answer.as_deref(),
            proposals[0].resolved_at.is_some(),
        ),
        (
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [{
                    "kind": "message",
                    "sessionId": "sample-session",
                    "content": indoc! {"
                        Proposal 1 \"Pick a format\": chose A (Format A).

                        Keep the simpler format.
                    "},
                }],
            }),
            Some(r#"{"choice":0,"comment":"Keep the simpler format."}"#),
            true,
        ),
    );
}

#[rstest]
#[case::claude(Engine::Claude, true)]
#[case::codex(Engine::Codex, false)]
#[tokio::test]
async fn proposal_answers_use_a_marker_only_for_claude(
    #[case] engine: Engine,
    #[case] expects_marker: bool,
    action_context: ActionContext,
) {
    let mut session = session("sample-session");
    session.engine = engine;
    action_context.save_session(&session);
    add_proposal(&action_context, "sample-session");

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/proposals/1/answer",
            json!({ "choice": null, "comment": null }),
        ))
        .await
        .unwrap();
    let sessions_dir = action_context.temp_dir.path().join("sessions");
    let marker = store::watch_web_user_marker_from(&sessions_dir, "sample-session").unwrap();
    let was_marked = action_context.runtime.calls.lock().unwrap()[0]["content"]
        .as_str()
        .is_some_and(|content| {
            marker.as_deref().is_some_and(|marker| {
                crate::commands::agent::watch_web_user_marker::strip_message(marker, content)
                    .is_some()
            })
        });

    assert_eq!(
        (
            response_snapshot(response, &action_context).await,
            marker.is_some(),
            was_marked,
        ),
        (
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [{
                    "kind": "message",
                    "sessionId": "sample-session",
                    "content": "Proposal 1 \"Pick a format\": none of the options fit. Propose another option.",
                }],
            }),
            expects_marker,
            expects_marker,
        ),
    );
}

#[rstest]
#[case::resolved_proposal(1, true, StatusCode::CONFLICT)]
#[case::choice_outside_options(2, false, StatusCode::BAD_REQUEST)]
#[tokio::test]
async fn unavailable_or_invalid_proposal_answers_are_rejected(
    #[case] choice: usize,
    #[case] resolve_first: bool,
    #[case] expected_status: StatusCode,
    action_context: ActionContext,
) {
    action_context.save_session(&session("sample-session"));
    add_proposal(&action_context, "sample-session");
    let sessions_dir = action_context.temp_dir.path().join("sessions");
    if resolve_first {
        store::close_unanswered_agent_proposals(&sessions_dir, "sample-session").unwrap();
    }

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/proposals/1/answer",
            json!({ "choice": choice, "comment": null }),
        ))
        .await
        .unwrap();
    let proposals = store::agent_proposals_from(&sessions_dir, "sample-session").unwrap();

    assert_eq!(
        (
            response_snapshot(response, &action_context).await["status"].as_u64(),
            proposals[0].answer.as_deref(),
            action_context.runtime.calls.lock().unwrap().len(),
        ),
        (Some(u64::from(expected_status.as_u16())), None, 0),
    );
}

#[rstest]
#[tokio::test]
async fn failed_peer_notification_does_not_store_the_answer(action_context: ActionContext) {
    action_context.save_session(&session("sample-session"));
    add_proposal(&action_context, "sample-session");
    action_context.set_message_error("peer notify failed");

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/proposals/1/answer",
            json!({ "choice": 0, "comment": null }),
        ))
        .await
        .unwrap();
    let proposals = store::agent_proposals_from(
        &action_context.temp_dir.path().join("sessions"),
        "sample-session",
    )
    .unwrap();

    assert_eq!(
        (
            response_snapshot(response, &action_context).await["status"].as_u64(),
            proposals[0].answer.as_deref(),
        ),
        (Some(u64::from(StatusCode::BAD_GATEWAY.as_u16())), None),
    );
}
