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
#[tokio::test]
async fn retry_removes_queued_prompt_when_message_delivery_fails(action_context: ActionContext) {
    action_context.save_session(&session("sample-session"));
    add_denied_operation(&action_context, "[Sensitive Operation]");
    action_context.set_message_error("message transport failed");
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
    let queued_prompts = store::watch_web_queued_prompts_from(
        &action_context.temp_dir.path().join("sessions"),
        "sample-session",
    )
    .unwrap();

    assert_eq!(
        (snapshot, queued_prompts),
        (
            json!({
                "status": StatusCode::BAD_GATEWAY.as_u16(),
                "body": { "error": "message transport failed" },
                "calls": [{
                    "kind": "message",
                    "sessionId": "sample-session",
                    "content": expected_message,
                }],
            }),
            Vec::<store::WatchWebQueuedPrompt>::new(),
        ),
    );
}

#[rstest]
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn concurrent_retries_send_the_operation_only_once(action_context: ActionContext) {
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
    let (router, retry_locks) = action_context.router_with_permission_denied_retry_locks();
    let (message_started, release_message) = action_context.block_next_message();
    let first_router = router.clone();
    let first = tokio::spawn(async move {
        first_router
            .oneshot(post(
                "/sessions/sample-session/permission-denials/1/retry",
                json!({}),
            ))
            .await
            .unwrap()
    });
    message_started.await.unwrap();

    let retry_lock = retry_locks.get_or_create("sample-session", 1).await;
    let second = tokio::spawn(async move {
        router
            .oneshot(post(
                "/sessions/sample-session/permission-denials/1/retry",
                json!({}),
            ))
            .await
            .unwrap()
    });
    let second_waited_for_first = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while std::sync::Arc::strong_count(&retry_lock) < 3 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .is_ok();
    let sent_before_first_completed = action_context.runtime.calls.lock().unwrap().len();
    let _ = release_message.send(());

    let first = response_snapshot(first.await.unwrap(), &action_context).await;
    let second = response_snapshot(second.await.unwrap(), &action_context).await;
    let operations = store::permission_denied_operations_from(
        &action_context.temp_dir.path().join("sessions"),
        "sample-session",
    )
    .unwrap();

    assert_eq!(
        (
            second_waited_for_first,
            sent_before_first_completed,
            first,
            second,
            operations
                .iter()
                .map(|operation| (operation.seq, operation.resolved_at.is_some()))
                .collect::<Vec<_>>(),
        ),
        (
            true,
            1,
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [{
                    "kind": "message",
                    "sessionId": "sample-session",
                    "content": expected_message,
                }],
            }),
            json!({
                "status": StatusCode::CONFLICT.as_u16(),
                "body": { "error": "denied operation was already retried" },
                "calls": [{
                    "kind": "message",
                    "sessionId": "sample-session",
                    "content": expected_message,
                }],
            }),
            vec![(1, true)],
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
