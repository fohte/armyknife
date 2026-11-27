use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use rstest::{fixture, rstest};
use serde_json::json;
use tower::ServiceExt;

use super::{ActionContext, action_context, post_empty, response_snapshot, session};
use crate::commands::agent::store;
use crate::commands::agent::types::SessionStatus;

#[fixture]
fn rewind_action_context(
    #[default(SessionStatus::Ended)] successor_status: SessionStatus,
    action_context: ActionContext,
) -> ActionContext {
    let mut replaced = session("fixture-replaced");
    replaced.status = SessionStatus::Ended;
    let mut successor = session("fixture-successor");
    successor.status = successor_status;
    store::save_rewind_sessions(
        &action_context.temp_dir.path().join("sessions"),
        &replaced,
        &successor,
    )
    .unwrap();
    action_context
}

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
async fn resume_endpoint_rejects_a_session_with_a_missing_working_directory(
    action_context: ActionContext,
) {
    let mut session = session("fixture-session");
    session.status = SessionStatus::Ended;
    session.cwd = action_context
        .temp_dir
        .path()
        .join("missing-working-directory");
    let working_directory = session.cwd.display().to_string();
    action_context.save_session(&session);

    let response = action_context
        .router()
        .oneshot(post_empty("/sessions/fixture-session/resume"))
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(response, &action_context).await,
        json!({
            "status": StatusCode::BAD_GATEWAY.as_u16(),
            "body": {
                "error": format!(
                    "session working directory does not exist: {working_directory}"
                ),
            },
            "calls": [],
        }),
    );
}

#[rstest]
#[tokio::test]
async fn resume_endpoint_rejects_a_replaced_id_when_its_successor_is_active(
    #[with(SessionStatus::Running)] rewind_action_context: ActionContext,
) {
    let response = rewind_action_context
        .router()
        .oneshot(post_empty("/sessions/fixture-replaced/resume"))
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(response, &rewind_action_context).await,
        json!({
            "status": StatusCode::CONFLICT.as_u16(),
            "body": { "error": "session is not ended or paused" },
            "calls": [],
        }),
    );
}

#[rstest]
#[tokio::test]
async fn actions_endpoint_uses_the_successor_status_for_a_replaced_id(
    #[with(SessionStatus::Running)] rewind_action_context: ActionContext,
) {
    let response = rewind_action_context
        .router()
        .oneshot(
            Request::builder()
                .uri("/sessions/fixture-replaced/actions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(response, &rewind_action_context).await,
        json!({
            "status": StatusCode::OK.as_u16(),
            "body": {
                "message": true,
                "answerQuestion": false,
                "resume": false,
                "interrupt": true,
                "openInTerminal": true,
                "rewind": false,
            },
            "calls": [],
        }),
    );
}

#[rstest]
#[tokio::test]
async fn repeated_resume_requests_for_a_replaced_id_share_the_successor_lock(
    #[with(SessionStatus::Ended)] rewind_action_context: ActionContext,
) {
    let app = rewind_action_context.router();

    let first = app
        .clone()
        .oneshot(post_empty("/sessions/fixture-replaced/resume"))
        .await
        .unwrap();
    let second = app
        .oneshot(post_empty("/sessions/fixture-replaced/resume"))
        .await
        .unwrap();

    let calls = json!([{
        "kind": "resume",
        "sessionId": "fixture-successor",
        "status": SessionStatus::Ended,
    }]);
    assert_eq!(
        (
            response_snapshot(first, &rewind_action_context).await,
            response_snapshot(second, &rewind_action_context).await,
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
                "resumeDisabledReason": "noTmux",
                "interrupt": false,
                "openInTerminal": false,
                "rewind": false,
            },
            "calls": [],
        }),
    );
}
