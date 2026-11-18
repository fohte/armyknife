use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use rstest::rstest;
use serde_json::json;
use tower::ServiceExt;

use super::{ActionContext, action_context, post_empty, response_snapshot, session};
use crate::commands::agent::store;
use crate::commands::agent::types::SessionStatus;

#[rstest]
#[case::ended(SessionStatus::Ended)]
#[case::paused(SessionStatus::Paused)]
#[tokio::test]
async fn resume_endpoint_delegates_ended_and_paused_sessions_to_the_runtime(
    #[case] status: SessionStatus,
    action_context: ActionContext,
) {
    let mut session = session("fixture-session");
    session.status = status;
    action_context.save_session(&session);

    let response = action_context
        .router()
        .oneshot(post_empty("/sessions/fixture-session/resume"))
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(response, &action_context).await,
        json!({
            "status": StatusCode::OK.as_u16(),
            "body": { "ok": true },
            "calls": [{
                "kind": "resume",
                "sessionId": "fixture-session",
                "status": status,
            }],
        }),
    );
}

#[rstest]
#[tokio::test]
async fn resume_endpoint_rejects_sessions_that_are_not_ended_or_paused(
    action_context: ActionContext,
) {
    action_context.save_session(&session("fixture-session"));

    let response = action_context
        .router()
        .oneshot(post_empty("/sessions/fixture-session/resume"))
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(response, &action_context).await,
        json!({
            "status": StatusCode::CONFLICT.as_u16(),
            "body": { "error": "session is not ended or paused" },
            "calls": [],
        }),
    );
}

#[rstest]
#[tokio::test]
async fn resume_endpoint_rejects_a_replaced_id_when_its_successor_is_active(
    action_context: ActionContext,
) {
    let mut replaced = session("fixture-replaced");
    replaced.status = SessionStatus::Ended;
    let mut successor = session("fixture-successor");
    successor.status = SessionStatus::Running;
    store::save_rewind_sessions(
        &action_context.temp_dir.path().join("sessions"),
        &replaced,
        &successor,
    )
    .unwrap();

    let response = action_context
        .router()
        .oneshot(post_empty("/sessions/fixture-replaced/resume"))
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(response, &action_context).await,
        json!({
            "status": StatusCode::CONFLICT.as_u16(),
            "body": { "error": "session is not ended or paused" },
            "calls": [],
        }),
    );
}

#[rstest]
#[tokio::test]
async fn resume_endpoint_rejects_a_second_request_until_the_session_starts(
    action_context: ActionContext,
) {
    let mut session = session("fixture-session");
    session.status = SessionStatus::Ended;
    action_context.save_session(&session);
    let app = action_context.router();

    let first = app
        .clone()
        .oneshot(post_empty("/sessions/fixture-session/resume"))
        .await
        .unwrap();
    let second = app
        .oneshot(post_empty("/sessions/fixture-session/resume"))
        .await
        .unwrap();

    let calls = json!([{
        "kind": "resume",
        "sessionId": "fixture-session",
        "status": SessionStatus::Ended,
    }]);
    assert_eq!(
        (
            response_snapshot(first, &action_context).await,
            response_snapshot(second, &action_context).await,
        ),
        (
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": calls.clone(),
            }),
            json!({
                "status": StatusCode::CONFLICT.as_u16(),
                "body": { "error": "session resume is already in progress" },
                "calls": calls,
            }),
        ),
    );
}

#[rstest]
#[tokio::test]
async fn ended_session_without_tmux_location_cannot_be_resumed(action_context: ActionContext) {
    let mut session = session("fixture-session");
    session.status = SessionStatus::Ended;
    session.tmux_info = None;
    action_context.save_session(&session);

    let response = action_context
        .router()
        .oneshot(
            Request::builder()
                .uri("/sessions/fixture-session/actions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(response, &action_context).await,
        json!({
            "status": StatusCode::OK.as_u16(),
            "body": {
                "message": false,
                "answerQuestion": false,
                "resume": false,
                "interrupt": false,
                "openInTerminal": false,
                "rewind": false,
            },
            "calls": [],
        }),
    );
}
