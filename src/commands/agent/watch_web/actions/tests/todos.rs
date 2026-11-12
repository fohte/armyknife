use axum::http::StatusCode;
use indoc::indoc;
use rstest::rstest;
use serde_json::json;
use tower::ServiceExt;

use super::{ActionContext, action_context, post, response_snapshot, session};
use crate::commands::agent::store;
use crate::commands::agent::types::SessionStatus;

#[rstest]
#[tokio::test]
async fn completing_a_todo_notifies_the_session_before_persisting(action_context: ActionContext) {
    action_context.save_session(&session("sample-session"));
    store::add_agent_todo_in(
        &action_context.temp_dir.path().join("sessions"),
        "sample-session",
        "Check the page",
        Some("Use a phone"),
    )
    .unwrap();

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/todos/1/complete",
            json!({ "comment": "The header fits." }),
        ))
        .await
        .unwrap();
    let todos = store::agent_todos_from(
        &action_context.temp_dir.path().join("sessions"),
        "sample-session",
    )
    .unwrap();

    assert_eq!(
        (
            response_snapshot(response, &action_context).await,
            todos
                .iter()
                .map(|todo| (
                    todo.title.as_str(),
                    todo.completed_at.is_some(),
                    todo.comment.as_deref(),
                ))
                .collect::<Vec<_>>(),
        ),
        (
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [{
                    "kind": "message",
                    "sessionId": "sample-session",
                    "content": indoc! {"
                        ユーザーが todo『Check the page』を済にした。

                        The header fits.
                    "}.trim_end(),
                }],
            }),
            vec![("Check the page", true, Some("The header fits."))],
        ),
    );
}

#[rstest]
#[tokio::test]
async fn completing_a_todo_for_an_ended_session_skips_notification(action_context: ActionContext) {
    let mut ended_session = session("sample-session");
    ended_session.status = SessionStatus::Ended;
    action_context.save_session(&ended_session);
    let sessions_dir = action_context.temp_dir.path().join("sessions");
    store::add_agent_todo_in(&sessions_dir, "sample-session", "Check the page", None).unwrap();

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/todos/1/complete",
            json!({ "comment": "The header fits." }),
        ))
        .await
        .unwrap();
    let todos = store::agent_todos_from(&sessions_dir, "sample-session").unwrap();

    assert_eq!(
        (
            response_snapshot(response, &action_context).await,
            todos
                .iter()
                .map(|todo| (
                    todo.title.as_str(),
                    todo.completed_at.is_some(),
                    todo.comment.as_deref(),
                ))
                .collect::<Vec<_>>(),
        ),
        (
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [],
            }),
            vec![("Check the page", true, Some("The header fits."))],
        ),
    );
}

#[rstest]
#[tokio::test]
async fn failed_session_notification_does_not_complete_the_todo(action_context: ActionContext) {
    action_context.save_session(&session("sample-session"));
    let sessions_dir = action_context.temp_dir.path().join("sessions");
    store::add_agent_todo_in(&sessions_dir, "sample-session", "Check the page", None).unwrap();
    action_context.set_message_error("peer notify failed");

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/todos/1/complete",
            json!({ "comment": "The header fits." }),
        ))
        .await
        .unwrap();
    let todos = store::agent_todos_from(&sessions_dir, "sample-session").unwrap();

    assert_eq!(
        (
            response_snapshot(response, &action_context).await,
            todos
                .iter()
                .map(|todo| (
                    todo.title.as_str(),
                    todo.completed_at.is_some(),
                    todo.comment.as_deref(),
                ))
                .collect::<Vec<_>>(),
        ),
        (
            json!({
                "status": StatusCode::BAD_GATEWAY.as_u16(),
                "body": { "error": "peer notify failed" },
                "calls": [{
                    "kind": "message",
                    "sessionId": "sample-session",
                    "content": indoc! {"
                        ユーザーが todo『Check the page』を済にした。

                        The header fits.
                    "}.trim_end(),
                }],
            }),
            vec![("Check the page", false, None)],
        ),
    );
}

#[rstest]
#[tokio::test]
async fn completed_todos_cannot_be_completed_again(action_context: ActionContext) {
    action_context.save_session(&session("sample-session"));
    let sessions_dir = action_context.temp_dir.path().join("sessions");
    let seq = store::add_agent_todo_in(&sessions_dir, "sample-session", "Task", None).unwrap();
    store::complete_agent_todo_from(&sessions_dir, "sample-session", seq, None).unwrap();

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/todos/1/complete",
            json!({ "comment": null }),
        ))
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(response, &action_context).await,
        json!({
            "status": StatusCode::CONFLICT.as_u16(),
            "body": { "error": "todo is not awaiting completion" },
            "calls": [],
        }),
    );
}
