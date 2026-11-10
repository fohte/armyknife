use std::path::PathBuf;

use axum::http::StatusCode;
use rstest::rstest;
use serde_json::json;
use tower::ServiceExt;

use super::{
    ActionContext, action_context, normalized_queued_prompts, post, response_snapshot, session,
};
use crate::commands::agent::store;
use crate::commands::agent::types::Engine;
use crate::commands::agent::watch_web::attachments;

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

    assert_eq!(
        (
            response_snapshot(response, &action_context).await,
            normalized_queued_prompts(&action_context, "sample-session", None),
        ),
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
        ),
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
    let marker_was_applied = marker
        .as_deref()
        .is_some_and(|marker| sent_content.starts_with(&format!("{marker}\n")));
    let snapshot = (
        response_snapshot(response, &action_context).await,
        normalized_queued_prompts(&action_context, "sample-session", None),
        marker.is_some(),
        marker_was_applied,
    );
    let expected_prompts = if engine == Engine::Claude {
        json!([{
            "id": "<prompt-id>",
            "content": "continue the investigation",
            "sentAt": "<sent-at>",
        }])
    } else {
        json!([])
    };
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
                "content": expected_message,
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
    let snapshot = || {
        json!({
            "response": response,
            "pathIsAbsolute": path.is_absolute(),
            "filename": filename.split_once("~image~").unwrap().1,
            "bytes": std::fs::read(&path).unwrap(),
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
