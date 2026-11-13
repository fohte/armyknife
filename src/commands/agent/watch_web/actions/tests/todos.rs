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
            "/sessions/sample-session/todos/1/resolve",
            json!({ "resolution": "completed", "comment": "The header fits." }),
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
                    todo.resolution,
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
                        <armyknife-todo-completed>The user marked the to-do as done: Check the page.

                        The header fits.</armyknife-todo-completed>
                    "}.trim_end(),
                }],
            }),
            vec![(
                "Check the page",
                Some(store::TodoResolution::Completed),
                Some("The header fits."),
            )],
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
            "/sessions/sample-session/todos/1/resolve",
            json!({ "resolution": "completed", "comment": "The header fits." }),
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
                    todo.resolution,
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
            vec![(
                "Check the page",
                Some(store::TodoResolution::Completed),
                Some("The header fits."),
            )],
        ),
    );
}

#[rstest]
#[tokio::test]
async fn rejecting_a_todo_notifies_the_session_and_persists_the_reason(
    action_context: ActionContext,
) {
    action_context.save_session(&session("sample-session"));
    let sessions_dir = action_context.temp_dir.path().join("sessions");
    store::add_agent_todo_in(&sessions_dir, "sample-session", "Check the page", None).unwrap();

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/todos/1/resolve",
            json!({ "resolution": "rejected", "comment": "No longer needed." }),
        ))
        .await
        .unwrap();
    let todos = store::agent_todos_from(&sessions_dir, "sample-session").unwrap();

    assert_eq!(
        (
            response_snapshot(response, &action_context).await,
            todos
                .iter()
                .map(|todo| (todo.resolution, todo.comment.as_deref()))
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
                        <armyknife-todo-rejected>The user rejected the to-do: Check the page.

                        No longer needed.</armyknife-todo-rejected>
                    "}.trim_end(),
                }],
            }),
            vec![(
                Some(store::TodoResolution::Rejected),
                Some("No longer needed."),
            )],
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
            "/sessions/sample-session/todos/1/resolve",
            json!({ "resolution": "completed", "comment": "The header fits." }),
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
                    todo.resolution,
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
                        <armyknife-todo-completed>The user marked the to-do as done: Check the page.

                        The header fits.</armyknife-todo-completed>
                    "}.trim_end(),
                }],
            }),
            vec![("Check the page", None, None)],
        ),
    );
}

#[rstest]
#[tokio::test]
async fn completed_todos_cannot_be_completed_again(action_context: ActionContext) {
    action_context.save_session(&session("sample-session"));
    let sessions_dir = action_context.temp_dir.path().join("sessions");
    let seq = store::add_agent_todo_in(&sessions_dir, "sample-session", "Task", None).unwrap();
    store::resolve_agent_todo_from(
        &sessions_dir,
        "sample-session",
        seq,
        store::TodoResolution::Completed,
        None,
    )
    .unwrap();

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/todos/1/resolve",
            json!({ "resolution": "completed", "comment": null }),
        ))
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(response, &action_context).await,
        json!({
            "status": StatusCode::CONFLICT.as_u16(),
            "body": { "error": "todo is not pending" },
            "calls": [],
        }),
    );
}

#[rstest]
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn concurrent_todo_resolutions_notify_only_the_persisted_winner(
    action_context: ActionContext,
) {
    action_context.save_session(&session("sample-session"));
    let sessions_dir = action_context.temp_dir.path().join("sessions");
    store::add_agent_todo_in(&sessions_dir, "sample-session", "Check the page", None).unwrap();
    let router = action_context.router();
    let (message_started, release_message) = action_context.block_next_message();
    let first_router = router.clone();
    let first = tokio::spawn(async move {
        first_router
            .oneshot(post(
                "/sessions/sample-session/todos/1/resolve",
                json!({ "resolution": "rejected", "comment": "No longer needed." }),
            ))
            .await
            .unwrap()
    });
    message_started.await.unwrap();

    let (second_started, second_started_receiver) = tokio::sync::oneshot::channel();
    let mut second = tokio::spawn(async move {
        let _ = second_started.send(());
        router
            .oneshot(post(
                "/sessions/sample-session/todos/1/resolve",
                json!({ "resolution": "completed", "comment": "Keep it." }),
            ))
            .await
            .unwrap()
    });
    second_started_receiver.await.unwrap();
    let second_finished_before_first =
        tokio::time::timeout(std::time::Duration::from_secs(1), &mut second)
            .await
            .is_ok();
    let calls_before_release = action_context.runtime.calls.lock().unwrap().len();
    let _ = release_message.send(());

    let first = response_snapshot(first.await.unwrap(), &action_context).await;
    let second_response = second.await.unwrap();
    let second = response_snapshot(second_response, &action_context).await;
    let todos = store::agent_todos_from(&sessions_dir, "sample-session").unwrap();

    assert_eq!(
        (
            second_finished_before_first,
            calls_before_release,
            first,
            second,
            todos
                .iter()
                .map(|todo| (todo.resolution, todo.comment.as_deref()))
                .collect::<Vec<_>>(),
        ),
        (
            false,
            1,
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [{
                    "kind": "message",
                    "sessionId": "sample-session",
                    "content": indoc! {"
                        <armyknife-todo-rejected>The user rejected the to-do: Check the page.

                        No longer needed.</armyknife-todo-rejected>
                    "}.trim_end(),
                }],
            }),
            json!({
                "status": StatusCode::CONFLICT.as_u16(),
                "body": { "error": "todo is not pending" },
                "calls": [{
                    "kind": "message",
                    "sessionId": "sample-session",
                    "content": indoc! {"
                        <armyknife-todo-rejected>The user rejected the to-do: Check the page.

                        No longer needed.</armyknife-todo-rejected>
                    "}.trim_end(),
                }],
            }),
            vec![(
                Some(store::TodoResolution::Rejected),
                Some("No longer needed."),
            )],
        ),
    );
}
