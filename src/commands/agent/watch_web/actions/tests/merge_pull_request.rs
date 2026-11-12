use axum::http::StatusCode;
use rstest::rstest;
use serde_json::json;
use tower::ServiceExt;

use super::{ActionContext, action_context, post, response_snapshot, session};
use crate::commands::gh::WatchWebPrMergeResult;

#[rstest]
#[case::merged(WatchWebPrMergeResult::Merged, json!({ "result": "merged" }))]
#[case::queued(
    WatchWebPrMergeResult::Enqueued("awaiting checks".to_string()),
    json!({ "result": "enqueued", "message": "awaiting checks" })
)]
#[tokio::test]
async fn merge_forwards_the_confirmed_head_and_returns_the_merge_result(
    #[case] result: WatchWebPrMergeResult,
    #[case] expected_body: serde_json::Value,
    action_context: ActionContext,
) {
    action_context.save_session(&session("sample-session"));
    *action_context.runtime.merge_result.lock().unwrap() = result;

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/pull-requests/42/merge",
            json!({
                "owner": "sample-org",
                "repo": "sample-repo",
                "expectedHeadSha": "confirmed-head",
            }),
        ))
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(response, &action_context).await,
        json!({
            "status": StatusCode::OK.as_u16(),
            "body": expected_body,
            "calls": [{
                "kind": "merge-pull-request",
                "owner": "sample-org",
                "repo": "sample-repo",
                "number": 42,
                "expectedHeadSha": "confirmed-head",
            }],
        }),
    );
}

#[rstest]
#[tokio::test]
async fn merge_errors_are_returned_with_the_failure_reason(action_context: ActionContext) {
    action_context.save_session(&session("sample-session"));
    *action_context.runtime.merge_error.lock().unwrap() =
        Some("PR #42 head changed since confirmation.".to_string());

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/pull-requests/42/merge",
            json!({
                "owner": "sample-org",
                "repo": "sample-repo",
                "expectedHeadSha": "confirmed-head",
            }),
        ))
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(response, &action_context).await,
        json!({
            "status": StatusCode::BAD_GATEWAY.as_u16(),
            "body": { "error": "PR #42 head changed since confirmation." },
            "calls": [{
                "kind": "merge-pull-request",
                "owner": "sample-org",
                "repo": "sample-repo",
                "number": 42,
                "expectedHeadSha": "confirmed-head",
            }],
        }),
    );
}

#[rstest]
#[tokio::test]
async fn merge_rejects_an_empty_confirmed_head_without_calling_runtime(
    action_context: ActionContext,
) {
    action_context.save_session(&session("sample-session"));

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/pull-requests/42/merge",
            json!({
                "owner": "sample-org",
                "repo": "sample-repo",
                "expectedHeadSha": " ",
            }),
        ))
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(response, &action_context).await,
        json!({
            "status": StatusCode::BAD_REQUEST.as_u16(),
            "body": { "error": "expectedHeadSha cannot be empty" },
            "calls": [],
        }),
    );
}

#[rstest]
#[tokio::test]
async fn merge_rejects_an_invalid_repository_without_calling_runtime(
    action_context: ActionContext,
) {
    action_context.save_session(&session("sample-session"));

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/pull-requests/42/merge",
            json!({
                "owner": "sample/org",
                "repo": "sample-repo",
                "expectedHeadSha": "confirmed-head",
            }),
        ))
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(response, &action_context).await,
        json!({
            "status": StatusCode::BAD_REQUEST.as_u16(),
            "body": { "error": "owner and repo must identify a canonical GitHub repository" },
            "calls": [],
        }),
    );
}

#[rstest]
#[tokio::test]
async fn merge_rejects_an_ended_session_without_calling_runtime(action_context: ActionContext) {
    let mut ended_session = session("sample-session");
    ended_session.status = crate::commands::agent::types::SessionStatus::Ended;
    action_context.save_session(&ended_session);

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/pull-requests/42/merge",
            json!({
                "owner": "sample-org",
                "repo": "sample-repo",
                "expectedHeadSha": "confirmed-head",
            }),
        ))
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(response, &action_context).await,
        json!({
            "status": StatusCode::CONFLICT.as_u16(),
            "body": { "error": "session has ended" },
            "calls": [],
        }),
    );
}
