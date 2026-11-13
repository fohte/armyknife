use axum::http::StatusCode;
use rstest::rstest;
use serde_json::json;
use tower::ServiceExt;

use super::{ActionContext, action_context, post, response_snapshot, session};
use crate::commands::agent::store;
use crate::commands::agent::types::SessionStatus;

fn add_denied_operation(context: &ActionContext, reason: &str) {
    store::add_permission_denied_from(
        &context.temp_dir.path().join("sessions"),
        "sample-session",
        Some("tool-call-fixture"),
        "Bash",
        &json!({ "command": "printf fixture" }),
        Some(reason),
    )
    .unwrap();
}

#[rstest]
#[tokio::test]
async fn retry_sends_one_time_user_message_and_completes_the_card(action_context: ActionContext) {
    action_context.save_session(&session("sample-session"));
    add_denied_operation(&action_context, "[Sensitive Operation]");
    let expected_operation = serde_json::to_string(&json!({
        "tool_name": "Bash",
        "tool_input": { "command": "printf fixture" },
        "tool_use_id": "tool-call-fixture",
    }))
    .unwrap();
    let expected_message = indoc::formatdoc! {"
        I approve retrying this exact blocked operation once.
        Please perform this tool call one time only:
        {expected_operation}
        This approval applies only to the operation shown above.
    "};

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/permission-denials/1/retry",
            json!({}),
        ))
        .await
        .unwrap();
    let snapshot = response_snapshot(response, &action_context).await;
    let operations = store::permission_denied_operations_from(
        &action_context.temp_dir.path().join("sessions"),
        "sample-session",
    )
    .unwrap();

    assert_eq!(
        (
            snapshot,
            operations
                .iter()
                .map(|operation| (operation.seq, operation.resolved_at.is_some()))
                .collect::<Vec<_>>(),
            store::load_session_from(
                &action_context.temp_dir.path().join("sessions"),
                "sample-session",
            )
            .unwrap()
            .unwrap()
            .status,
        ),
        (
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [{
                    "kind": "message",
                    "sessionId": "sample-session",
                    "content": expected_message,
                }],
            }),
            vec![(1, true)],
            SessionStatus::WaitingInput,
        ),
    );
}

#[rstest]
#[case::missing_verdict("Auto mode could not evaluate this action and is blocking it for safety.")]
#[case::classifier_unavailable("Classifier unavailable")]
#[case::no_reason("No reason provided")]
#[tokio::test]
async fn no_verdict_denial_cannot_be_retried(#[case] reason: &str, action_context: ActionContext) {
    action_context.save_session(&session("sample-session"));
    add_denied_operation(&action_context, reason);

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/permission-denials/1/retry",
            json!({}),
        ))
        .await
        .unwrap();
    let snapshot = response_snapshot(response, &action_context).await;
    let operations = store::permission_denied_operations_from(
        &action_context.temp_dir.path().join("sessions"),
        "sample-session",
    )
    .unwrap();

    assert_eq!(
        (
            snapshot,
            operations
                .iter()
                .map(|operation| operation.resolved_at.is_some())
                .collect::<Vec<_>>(),
        ),
        (
            json!({
                "status": StatusCode::CONFLICT.as_u16(),
                "body": { "error": "this denial has no classifier verdict and cannot be approved" },
                "calls": [],
            }),
            vec![false],
        ),
    );
}
