use std::collections::HashMap;
use std::path::PathBuf;

use axum::http::StatusCode;
use chrono::Utc;
use rstest::rstest;
use serde_json::json;
use tower::ServiceExt;

use super::super::messages::lock_session_action;
use super::{
    ActionContext, action_context, normalized_queued_prompts, post, response_snapshot, session,
};
use crate::commands::agent::store;
use crate::commands::agent::types::Engine;
use crate::commands::agent::watch_web::attachments;

fn normalize_message_ids(ids: Vec<String>, labels: &mut HashMap<String, String>) -> Vec<String> {
    ids.into_iter()
        .map(|id| {
            let label = format!("<message-id-{}>", labels.len() + 1);
            labels.entry(id).or_insert(label).clone()
        })
        .collect()
}

#[rstest]
#[tokio::test]
async fn send_message_forwards_nonempty_content_unchanged(action_context: ActionContext) {
    action_context.save_session(&session("sample-session"));

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/messages",
            json!({ "content": "  continue the investigation  " }),
        ))
        .await
        .unwrap();

    let sessions_dir = action_context.temp_dir.path().join("sessions");
    let sent_at = store::watch_web_queued_prompts_from(&sessions_dir, "sample-session")
        .unwrap()
        .first()
        .unwrap()
        .sent_at;
    let actual = (
        response_snapshot(response, &action_context).await,
        normalized_queued_prompts(&action_context, "sample-session", None),
        action_context
            .load_session("sample-session")
            .last_prompted_at,
    );
    assert_eq!(
        actual,
        (
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [{
                    "kind": "message",
                    "sessionId": "sample-session",
                    "content": "  continue the investigation  ",
                }],
            }),
            json!([{
                "id": "<prompt-id>",
                "content": "  continue the investigation  ",
                "sentAt": "<sent-at>",
            }]),
            Some(sent_at),
        ),
    );
}

#[rstest]
#[tokio::test]
async fn send_message_keeps_a_newer_prompt_timestamp(action_context: ActionContext) {
    action_context.save_session(&session("sample-session"));
    let later_prompt_at = Utc::now() + chrono::Duration::seconds(1);
    let callback_prompt_at = later_prompt_at;
    let sessions_dir = action_context.temp_dir.path().join("sessions");
    action_context.set_after_message(move || {
        store::update_session_in(&sessions_dir, "sample-session", |session| {
            session.last_prompted_at = Some(callback_prompt_at);
            true
        })
        .unwrap();
    });

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/messages",
            json!({ "content": "continue the investigation" }),
        ))
        .await
        .unwrap();

    let actual = (
        response_snapshot(response, &action_context).await,
        normalized_queued_prompts(&action_context, "sample-session", None),
        action_context
            .load_session("sample-session")
            .last_prompted_at,
    );
    assert_eq!(
        actual,
        (
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [{
                    "kind": "message",
                    "sessionId": "sample-session",
                    "content": "continue the investigation",
                }],
            }),
            json!([{
                "id": "<prompt-id>",
                "content": "continue the investigation",
                "sentAt": "<sent-at>",
            }]),
            Some(later_prompt_at),
        ),
    );
}

#[rstest]
#[tokio::test]
async fn send_message_stays_successful_when_prompt_timestamp_cannot_be_saved(
    action_context: ActionContext,
) {
    action_context.save_session(&session("sample-session"));
    let sessions_dir = action_context.temp_dir.path().join("sessions");
    action_context.set_after_message(move || {
        let database_path = sessions_dir.join("sessions.db");
        std::fs::remove_file(&database_path).unwrap();
        std::fs::create_dir(&database_path).unwrap();
    });

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/messages",
            json!({ "content": "continue the investigation" }),
        ))
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(response, &action_context).await,
        json!({
            "status": StatusCode::OK.as_u16(),
            "body": { "ok": true },
            "calls": [{
                "kind": "message",
                "sessionId": "sample-session",
                "content": "continue the investigation",
            }],
        }),
    );
}

#[rstest]
#[tokio::test]
async fn send_message_rejects_a_session_marked_for_shutdown(action_context: ActionContext) {
    let mut session = session("sample-session");
    session.sweep_signaled = true;
    action_context.save_session(&session);

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/messages",
            json!({ "content": "continue the investigation" }),
        ))
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(response, &action_context).await,
        json!({
            "status": StatusCode::CONFLICT.as_u16(),
            "body": { "error": "session is being stopped" },
            "calls": [],
        }),
    );
}

#[rstest]
#[tokio::test]
async fn send_message_rechecks_the_session_after_waiting_for_a_rewind_action(
    action_context: ActionContext,
) {
    action_context.save_session(&session("sample-session"));
    let action_guard = lock_session_action("sample-session").await;
    let router = action_context.router();
    let request = tokio::spawn(async move {
        router
            .oneshot(post(
                "/sessions/sample-session/messages",
                json!({ "content": "continue the investigation" }),
            ))
            .await
            .unwrap()
    });
    tokio::task::yield_now().await;

    let mut session = action_context.load_session("sample-session");
    session.sweep_signaled = true;
    action_context.save_session(&session);
    drop(action_guard);

    assert_eq!(
        response_snapshot(request.await.unwrap(), &action_context).await,
        json!({
            "status": StatusCode::CONFLICT.as_u16(),
            "body": { "error": "session is being stopped" },
            "calls": [],
        }),
    );
}

#[rstest]
#[case::claude_with_marker(Engine::Claude, true)]
#[case::legacy_claude_without_marker(Engine::Claude, false)]
#[case::codex(Engine::Codex, false)]
#[tokio::test]
async fn send_message_ensures_markers_for_claude_sessions(
    #[case] engine: Engine,
    #[case] marker_exists_before: bool,
    action_context: ActionContext,
) {
    let mut session = session("sample-session");
    session.engine = engine;
    action_context.save_session(&session);
    let sessions_dir = action_context.temp_dir.path().join("sessions");
    if marker_exists_before {
        store::lock_session_for_update(&sessions_dir, &session.session_id)
            .unwrap()
            .ensure_watch_web_user_marker()
            .unwrap();
    }

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/messages",
            json!({ "content": "continue the investigation" }),
        ))
        .await
        .unwrap();

    let marker = if engine == Engine::Claude {
        store::watch_web_user_marker_from(&sessions_dir, &session.session_id).unwrap()
    } else {
        None
    };
    let sent_content = action_context.runtime.calls.lock().unwrap()[0]["content"]
        .as_str()
        .unwrap()
        .to_string();
    let marker_was_applied = marker.as_deref().is_some_and(|marker| {
        crate::commands::agent::watch_web_user_marker::strip_message(marker, &sent_content)
            .is_some()
    });
    let snapshot = (
        response_snapshot(response, &action_context).await,
        normalized_queued_prompts(&action_context, "sample-session", None),
        marker.is_some(),
        marker_was_applied,
    );
    let expected_prompts = json!([{
        "id": "<prompt-id>",
        "content": "continue the investigation",
        "sentAt": "<sent-at>",
    }]);
    assert_eq!(
        snapshot,
        (
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [{
                    "kind": "message",
                    "sessionId": "sample-session",
                    "content": "continue the investigation",
                }],
            }),
            expected_prompts,
            engine == Engine::Claude,
            engine == Engine::Claude,
        ),
    );
}

#[rstest]
#[case::claude(Engine::Claude)]
#[case::codex(Engine::Codex)]
#[tokio::test]
async fn duplicate_messages_keep_their_ids_and_structured_attachments(
    #[case] engine: Engine,
    action_context: ActionContext,
) {
    let mut session = session("sample-session");
    session.engine = engine;
    action_context.save_session(&session);
    let request = json!({
        "content": "Repeat this request.",
        "attachments": [{
            "name": "notes.txt",
            "mimeType": "text/plain",
            "size": 5,
            "data": "aGVsbG8=",
        }],
    });
    let mut statuses = Vec::new();
    for _ in 0..2 {
        let response = action_context
            .router()
            .oneshot(post("/sessions/sample-session/messages", request.clone()))
            .await
            .unwrap();
        statuses.push(response_snapshot(response, &action_context).await["status"].clone());
    }

    let sent_ids = action_context
        .runtime
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter_map(|call| call["content"].as_str())
        .filter_map(|content| {
            content
                .rsplit_once("<!-- armyknife-queued-prompt-id:")
                .and_then(|(_, marker)| marker.strip_suffix(" -->"))
                .map(str::to_owned)
        })
        .collect::<Vec<_>>();
    let sessions_dir = action_context.temp_dir.path().join("sessions");
    let prompts = store::watch_web_queued_prompts_from(&sessions_dir, "sample-session").unwrap();
    let persisted_ids = prompts
        .iter()
        .filter_map(|prompt| prompt.message_id.clone())
        .collect::<Vec<_>>();
    let prompt_records = prompts
        .iter()
        .map(|prompt| {
            (
                prompt.content.clone(),
                prompt
                    .attachments
                    .iter()
                    .map(|attachment| match attachment {
                        store::WatchWebQueuedAttachment::File { name } => name.clone(),
                        store::WatchWebQueuedAttachment::Image { source } => source.clone(),
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();

    let mut id_labels = HashMap::new();
    let normalized_ids = (
        normalize_message_ids(sent_ids, &mut id_labels),
        normalize_message_ids(persisted_ids, &mut id_labels),
    );
    assert_eq!(
        (normalized_ids, statuses, prompt_records),
        (
            (
                vec!["<message-id-1>".to_owned(), "<message-id-2>".to_owned()],
                vec!["<message-id-1>".to_owned(), "<message-id-2>".to_owned()],
            ),
            vec![
                json!(StatusCode::OK.as_u16()),
                json!(StatusCode::OK.as_u16()),
            ],
            vec![
                (
                    "Repeat this request.".to_owned(),
                    vec!["notes.txt".to_owned()],
                ),
                (
                    "Repeat this request.".to_owned(),
                    vec!["notes.txt".to_owned()],
                ),
            ],
        ),
    );
}

#[rstest]
#[case::custom_message("Inspect this file")]
#[case::empty_message("")]
#[case::whitespace_message("   ")]
#[tokio::test]
async fn send_message_stores_file_and_forwards_its_absolute_path(
    #[case] content: &str,
    action_context: ActionContext,
) {
    action_context.save_session(&session("sample-session"));
    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/messages",
            json!({
                "content": content,
                "attachments": [{
                    "name": "design.html",
                    "mimeType": "text/html",
                    "size": 9,
                    "data": "TW9jayBIVE1M",
                }],
            }),
        ))
        .await
        .unwrap();

    let attachment_dir = action_context.temp_dir.path().join("attachments");
    let path = std::fs::read_dir(&attachment_dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .canonicalize()
        .unwrap();
    let mut response_snapshot = response_snapshot(response, &action_context).await;
    let path_string = path.display().to_string();
    let references_saved_path = response_snapshot["calls"][0]["content"]
        .as_str()
        .is_some_and(|content| content.trim_end().ends_with(&path_string));
    let message = response_snapshot["calls"][0]["content"]
        .as_str()
        .unwrap()
        .replace(&path_string, "<attachment-path>");
    response_snapshot["calls"][0]["content"] = message.into();
    let snapshot = || {
        json!({
            "response": response_snapshot,
            "queuedPrompts": normalized_queued_prompts(
                &action_context,
                "sample-session",
                Some(&path_string),
            ),
            "referencesSavedPath": references_saved_path,
            "attachment": {
                "isAbsolute": path.is_absolute(),
                "bytes": std::fs::read(&path).unwrap(),
            },
        })
    };
    let expected_message = if content.trim().is_empty() {
        indoc::indoc! {"
            Please inspect the attached file(s):
            - <attachment-path>
        "}
        .trim_end()
        .to_string()
    } else {
        format!(
            indoc::indoc! {"
                {}

                Attached file(s):
                - <attachment-path>
            "},
            content
        )
        .trim_end()
        .to_string()
    };
    assert_eq!(
        snapshot(),
        json!({
            "response": {
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [{
                    "kind": "message",
                    "sessionId": "sample-session",
                    "content": expected_message,
                }],
            },
            "queuedPrompts": [{
                "id": "<prompt-id>",
                "content": content,
                "sentAt": "<sent-at>",
            }],
            "referencesSavedPath": true,
            "attachment": {
                "isAbsolute": true,
                "bytes": [77, 111, 99, 107, 32, 72, 84, 77, 76],
            },
        }),
    );
}

#[rstest]
#[tokio::test]
async fn send_message_accepts_legacy_image_payloads_during_page_restarts(
    action_context: ActionContext,
) {
    action_context.save_session(&session("sample-session"));
    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/messages",
            json!({
                "content": "",
                "images": [{
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
    let mut response = response_snapshot(response, &action_context).await;
    let path_string = path.display().to_string();
    response["calls"][0]["content"] = response["calls"][0]["content"]
        .as_str()
        .unwrap()
        .replace(&path_string, "<attachment-path>")
        .into();
    let filename = path.file_name().unwrap().to_string_lossy();
    let prompts = store::watch_web_queued_prompts_from(
        &action_context.temp_dir.path().join("sessions"),
        "sample-session",
    )
    .unwrap();
    let prompt = prompts.first().unwrap();
    let attachments = prompt
        .attachments
        .iter()
        .map(|attachment| match attachment {
            store::WatchWebQueuedAttachment::Image { source } => json!({
                "type": "image",
                "source": source.replace(filename.as_ref(), "<image-file>"),
            }),
            store::WatchWebQueuedAttachment::File { name } => json!({
                "type": "file",
                "name": name,
            }),
        })
        .collect::<Vec<_>>();
    let snapshot = || {
        json!({
            "response": response,
            "pathIsAbsolute": path.is_absolute(),
            "filename": filename.split_once("~image~").unwrap().1,
            "bytes": std::fs::read(&path).unwrap(),
            "queuedPrompt": {
                "content": prompt.content,
                "attachments": attachments,
            },
        })
    };

    assert_eq!(
        snapshot(),
        json!({
            "response": {
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [{
                    "kind": "message",
                    "sessionId": "sample-session",
                    "content": "Please inspect the attached file(s):\n- <attachment-path>",
                }],
            },
            "pathIsAbsolute": true,
            "filename": "legacy-image.gif",
            "bytes": [71, 73, 70, 56, 57, 97],
            "queuedPrompt": {
                "content": "",
                "attachments": [{
                    "type": "image",
                    "source": "/api/images/<image-file>",
                }],
            },
        }),
    );
}

#[rstest]
#[tokio::test]
async fn send_message_removes_saved_files_when_runtime_fails(action_context: ActionContext) {
    action_context.save_session(&session("sample-session"));
    action_context.set_message_error("message transport failed");
    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/messages",
            json!({
                "content": "Inspect this file",
                "attachments": [{
                    "name": "design.html",
                    "mimeType": "text/html",
                    "size": 9,
                    "data": "TW9jayBIVE1M",
                }],
            }),
        ))
        .await
        .unwrap();

    let mut response = response_snapshot(response, &action_context).await;
    let message = response["calls"][0]["content"]
        .as_str()
        .unwrap()
        .to_string();
    let path_line = message
        .lines()
        .rev()
        .find(|line| line.starts_with("- "))
        .unwrap_or_default();
    let path = path_line.strip_prefix("- ").unwrap_or_default();
    let path_is_absolute = PathBuf::from(path).is_absolute();
    response["calls"][0]["content"] = message.replace(path, "<attachment-path>").into();
    let files_left = std::fs::read_dir(action_context.temp_dir.path().join("attachments"))
        .map(|entries| entries.count())
        .unwrap_or_default();
    let actual = json!({
        "response": response,
        "queuedPrompts": normalized_queued_prompts(
            &action_context,
            "sample-session",
            None,
        ),
        "pathIsAbsolute": path_is_absolute,
        "filesLeft": files_left,
    });
    let expected_message = indoc::indoc! {"
        Inspect this file

        Attached file(s):
        - <attachment-path>
    "}
    .trim_end();
    assert_eq!(
        actual,
        json!({
            "response": {
                "status": StatusCode::BAD_GATEWAY.as_u16(),
                "body": { "error": "message transport failed" },
                "calls": [{
                    "kind": "message",
                    "sessionId": "sample-session",
                    "content": expected_message,
                }],
            },
            "queuedPrompts": [],
            "pathIsAbsolute": true,
            "filesLeft": 0,
        }),
    );
}

#[rstest]
#[case::oversized(
    vec![("large.bin", attachments::MAX_ATTACHMENT_BYTES + 1)],
    "file attachments exceed the 50 MiB limit"
)]
#[case::too_many_attachments(
    vec![("file.bin", 0); attachments::MAX_ATTACHMENTS_PER_MESSAGE + 1],
    "a message cannot include more than 16 file attachments"
)]
#[case::cumulative_size(
    vec![("large-a.bin", 30 * 1024 * 1024), ("large-b.bin", 21 * 1024 * 1024)],
    "file attachments exceed the 50 MiB limit"
)]
#[tokio::test]
async fn send_message_rejects_invalid_file_attachments(
    #[case] attachments: Vec<(&str, usize)>,
    #[case] error: &str,
    action_context: ActionContext,
) {
    action_context.save_session(&session("sample-session"));

    let files = attachments
        .iter()
        .map(|(name, size)| json!({ "name": name, "mimeType": "application/octet-stream", "size": size, "data": "" }))
        .collect::<Vec<_>>();
    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/messages",
            json!({
                "content": "Inspect this file",
                "attachments": files,
            }),
        ))
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(response, &action_context).await,
        json!({
            "status": StatusCode::BAD_REQUEST.as_u16(),
            "body": { "error": error },
            "calls": [],
        }),
    );
}

#[rstest]
#[tokio::test]
async fn send_message_rejects_whitespace_without_calling_runtime(action_context: ActionContext) {
    action_context.save_session(&session("sample-session"));

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/messages",
            json!({ "content": "  \n " }),
        ))
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(response, &action_context).await,
        json!({
            "status": StatusCode::BAD_REQUEST.as_u16(),
            "body": { "error": "message must not be empty" },
            "calls": [],
        }),
    );
}
#[rstest]
#[case::not_registered("not-stored")]
#[case::invalid_id("bad..id")]
#[tokio::test]
async fn unknown_session_is_not_found_without_calling_runtime(
    #[case] id: &str,
    action_context: ActionContext,
) {
    let response = action_context
        .router()
        .oneshot(post(
            &format!("/sessions/{id}/messages"),
            json!({ "content": "hello" }),
        ))
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(response, &action_context).await,
        json!({
            "status": StatusCode::NOT_FOUND.as_u16(),
            "body": { "error": "session not found" },
            "calls": [],
        }),
    );
}
