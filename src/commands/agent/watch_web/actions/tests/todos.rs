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
async fn folds_a_pending_todo_without_erasing_its_added_folded_marker(
    action_context: ActionContext,
) {
    action_context.save_session(&session("sample-session"));
    store::add_agent_todo_in(
        &action_context.temp_dir.path().join("sessions"),
        "sample-session",
        "Check the alert sound",
        None,
        true,
    )
    .unwrap();

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/todos/1/fold",
            json!({ "folded": false }),
        ))
        .await
        .unwrap();
    let todo = store::agent_todos_from(
        &action_context.temp_dir.path().join("sessions"),
        "sample-session",
    )
    .unwrap()
    .into_iter()
    .next()
    .map(|todo| (todo.folded, todo.added_folded));

    assert_eq!(
        (response_snapshot(response, &action_context).await, todo),
        (
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [],
            }),
            Some((false, true)),
        ),
    );
}

#[rstest]
#[tokio::test]
async fn folding_a_completed_todo_returns_conflict(action_context: ActionContext) {
    action_context.save_session(&session("sample-session"));
    let sessions_dir = action_context.temp_dir.path().join("sessions");
    let seq = store::add_agent_todo_in(
        &sessions_dir,
        "sample-session",
        "Check the alert sound",
        None,
        false,
    )
    .unwrap();
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
            &format!("/sessions/sample-session/todos/{seq}/fold"),
            json!({ "folded": true }),
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
#[tokio::test]
async fn completing_a_todo_notifies_the_session_before_persisting(action_context: ActionContext) {
    action_context.save_session(&session("sample-session"));
    store::add_agent_todo_in(
        &action_context.temp_dir.path().join("sessions"),
        "sample-session",
        "Check the page",
        Some("Use a phone"),
        false,
    )
    .unwrap();

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/todos/1/resolve",
            json!({ "resolution": "completed", "comment": "The header fits.", "expectedTitle": "Check the page", "expectedDetail": "Use a phone" }),
        ))
        .await
        .unwrap();
    let todos = store::agent_todos_from(
        &action_context.temp_dir.path().join("sessions"),
        "sample-session",
    )
    .unwrap();
    let sent_content = action_context.runtime.calls.lock().unwrap()[0]["content"]
        .as_str()
        .unwrap()
        .to_owned();
    let action_message_id =
        crate::commands::agent::watch_web::messages::queued_prompt::id_from_content(&sent_content)
            .is_some_and(|message_id| message_id.starts_with("armyknife-action:"));

    assert_eq!(
        (
            response_snapshot(response, &action_context).await,
            action_message_id,
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
            true,
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
async fn todo_resolution_without_expected_content_is_accepted(action_context: ActionContext) {
    action_context.save_session(&session("sample-session"));
    let sessions_dir = action_context.temp_dir.path().join("sessions");
    store::add_agent_todo_in(
        &sessions_dir,
        "sample-session",
        "Check the page",
        Some("Use a phone"),
        false,
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
    let todos = store::agent_todos_from(&sessions_dir, "sample-session").unwrap();

    assert_eq!(
        (
            response_snapshot(response, &action_context).await,
            todos
                .iter()
                .map(|todo| (
                    todo.title.as_str(),
                    todo.detail.as_deref(),
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
                    "content": "<armyknife-todo-completed>The user marked the to-do as done: Check the page.</armyknife-todo-completed>",
                }],
            }),
            vec![(
                "Check the page",
                Some("Use a phone"),
                Some(store::TodoResolution::Completed),
                None,
            )],
        ),
    );
}

#[rstest]
#[tokio::test]
async fn stale_todo_resolution_is_rejected_after_the_todo_is_edited(action_context: ActionContext) {
    action_context.save_session(&session("sample-session"));
    let sessions_dir = action_context.temp_dir.path().join("sessions");
    let seq = store::add_agent_todo_in(
        &sessions_dir,
        "sample-session",
        "Check the page",
        Some("Use a phone"),
        false,
    )
    .unwrap();
    store::edit_agent_todo_from(
        &sessions_dir,
        "sample-session",
        seq,
        "Review the page",
        Some("Check the navigation"),
        false,
    )
    .unwrap();

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/todos/1/resolve",
            json!({
                "resolution": "completed",
                "comment": null,
                "expectedTitle": "Check the page",
                "expectedDetail": "Use a phone",
            }),
        ))
        .await
        .unwrap();
    let todos = store::agent_todos_from(&sessions_dir, "sample-session").unwrap();
    let calls = action_context.runtime.calls.lock().unwrap().clone();

    assert_eq!(
        (
            response_snapshot(response, &action_context).await,
            todos
                .iter()
                .map(|todo| (
                    todo.seq,
                    todo.title.as_str(),
                    todo.detail.as_deref(),
                    todo.completed_at,
                    todo.resolution,
                    todo.comment.as_deref(),
                ))
                .collect::<Vec<_>>(),
            calls,
        ),
        (
            json!({
                "status": StatusCode::CONFLICT.as_u16(),
                "body": { "error": "todo changed; reload it before resolving" },
                "calls": [],
            }),
            vec![(
                seq,
                "Review the page",
                Some("Check the navigation"),
                None,
                None,
                None,
            )],
            Vec::new(),
        ),
    );
}

#[rstest]
#[tokio::test]
async fn resolving_a_todo_saves_and_forwards_image_attachments(action_context: ActionContext) {
    action_context.save_session(&session("sample-session"));
    store::add_agent_todo_in(
        &action_context.temp_dir.path().join("sessions"),
        "sample-session",
        "Check the page",
        None,
        false,
    )
    .unwrap();

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/todos/1/resolve",
            json!({
                "resolution": "completed",
                "comment": "The header fits.",
                "expectedTitle": "Check the page",
                "expectedDetail": null,
                "attachments": [{
                    "name": "screen.gif",
                    "mimeType": "image/gif",
                    "size": 6,
                    "data": "R0lGODlh",
                }],
            }),
        ))
        .await
        .unwrap();
    let directory = action_context.temp_dir.path().join("attachments");
    let path = std::fs::read_dir(&directory)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .canonicalize()
        .unwrap();
    let path_string = path.display().to_string();
    let image_source = format!(
        "/api/images/{}",
        path.file_name().unwrap().to_string_lossy()
    );
    let todos = store::agent_todos_from(
        &action_context.temp_dir.path().join("sessions"),
        "sample-session",
    )
    .unwrap();
    let stored_comment = todos[0]
        .comment
        .as_deref()
        .unwrap_or_default()
        .replace(&image_source, "<attachment-source>");
    let mut response = response_snapshot(response, &action_context).await;
    response["calls"][0]["content"] = response["calls"][0]["content"]
        .as_str()
        .unwrap()
        .replace(&path_string, "<attachment-path>")
        .into();

    assert_eq!(
        (response, std::fs::read(path).unwrap(), stored_comment,),
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

                        Attached file(s):
                        - <attachment-path>
                    "}.trim_end(),
                }],
            }),
            vec![71, 73, 70, 56, 57, 97],
            indoc! {"
                The header fits.

                ![screen.gif](<attachment-source>)
            "}
            .trim_end()
            .to_string(),
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
    store::add_agent_todo_in(
        &sessions_dir,
        "sample-session",
        "Check the page",
        None,
        false,
    )
    .unwrap();

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/todos/1/resolve",
            json!({ "resolution": "completed", "comment": "The header fits.", "expectedTitle": "Check the page", "expectedDetail": null }),
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
async fn ended_session_rejects_todo_attachments(action_context: ActionContext) {
    let mut ended_session = session("sample-session");
    ended_session.status = SessionStatus::Ended;
    action_context.save_session(&ended_session);
    let sessions_dir = action_context.temp_dir.path().join("sessions");
    store::add_agent_todo_in(
        &sessions_dir,
        "sample-session",
        "Check the page",
        None,
        false,
    )
    .unwrap();

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/todos/1/resolve",
            json!({
                "resolution": "completed",
                "comment": "The header fits.",
                "expectedTitle": "Check the page",
                "expectedDetail": null,
                "attachments": [{
                    "name": "screen.gif",
                    "mimeType": "image/gif",
                    "size": 6,
                    "data": "R0lGODlh",
                }],
            }),
        ))
        .await
        .unwrap();
    let todos = store::agent_todos_from(&sessions_dir, "sample-session").unwrap();
    let files_left = std::fs::read_dir(action_context.temp_dir.path().join("attachments"))
        .map(|entries| entries.count())
        .unwrap_or_default();

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
            files_left,
        ),
        (
            json!({
                "status": StatusCode::CONFLICT.as_u16(),
                "body": { "error": "session has ended" },
                "calls": [],
            }),
            vec![("Check the page", None, None)],
            0,
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
    store::add_agent_todo_in(
        &sessions_dir,
        "sample-session",
        "Check the page",
        None,
        false,
    )
    .unwrap();

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/todos/1/resolve",
            json!({ "resolution": "rejected", "comment": "No longer needed.", "expectedTitle": "Check the page", "expectedDetail": null }),
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
    store::add_agent_todo_in(
        &sessions_dir,
        "sample-session",
        "Check the page",
        None,
        false,
    )
    .unwrap();
    action_context.set_message_error("peer notify failed");

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/todos/1/resolve",
            json!({
                "resolution": "completed",
                "comment": "The header fits.",
                "expectedTitle": "Check the page",
                "expectedDetail": null,
                "attachments": [{
                    "name": "screen.gif",
                    "mimeType": "image/gif",
                    "size": 6,
                    "data": "R0lGODlh",
                }],
            }),
        ))
        .await
        .unwrap();
    let todos = store::agent_todos_from(&sessions_dir, "sample-session").unwrap();
    let mut response = response_snapshot(response, &action_context).await;
    let message = response["calls"][0]["content"].as_str().unwrap();
    let path = message
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix("- "))
        .unwrap_or_default();
    response["calls"][0]["content"] = message.replace(path, "<attachment-path>").into();
    let files_left = std::fs::read_dir(action_context.temp_dir.path().join("attachments"))
        .map(|entries| entries.count())
        .unwrap_or_default();

    assert_eq!(
        (
            response,
            todos
                .iter()
                .map(|todo| (
                    todo.title.as_str(),
                    todo.resolution,
                    todo.comment.as_deref(),
                ))
                .collect::<Vec<_>>(),
            files_left,
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

                        Attached file(s):
                        - <attachment-path>
                    "}.trim_end(),
                }],
            }),
            vec![("Check the page", None, None)],
            0,
        ),
    );
}

#[rstest]
#[tokio::test]
async fn completed_todos_cannot_be_completed_again(action_context: ActionContext) {
    action_context.save_session(&session("sample-session"));
    let sessions_dir = action_context.temp_dir.path().join("sessions");
    let seq =
        store::add_agent_todo_in(&sessions_dir, "sample-session", "Task", None, false).unwrap();
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
            json!({ "resolution": "completed", "comment": null, "expectedTitle": "Task", "expectedDetail": null }),
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
    store::add_agent_todo_in(
        &sessions_dir,
        "sample-session",
        "Check the page",
        None,
        false,
    )
    .unwrap();
    let router = action_context.router();
    let (message_started, release_message) = action_context.block_next_message();
    let first_router = router.clone();
    let first = tokio::spawn(async move {
        first_router
            .oneshot(post(
                "/sessions/sample-session/todos/1/resolve",
                json!({ "resolution": "rejected", "comment": "No longer needed.", "expectedTitle": "Check the page", "expectedDetail": null }),
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
                json!({ "resolution": "completed", "comment": "Keep it.", "expectedTitle": "Check the page", "expectedDetail": null }),
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
