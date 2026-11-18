use axum::http::StatusCode;
use rstest::rstest;
use serde_json::json;
use tower::ServiceExt;

use super::{ActionContext, action_context, post_empty, response_snapshot, session};
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
