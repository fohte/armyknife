use std::path::PathBuf;

use axum::http::StatusCode;
use indoc::indoc;
use rstest::rstest;
use serde_json::json;
use tower::ServiceExt;

use super::super::CommandResult;
use super::{
    ActionContext, action_context, normalized_queued_prompts, post, response_snapshot, session,
};
use crate::commands::agent::types::{Engine, Session, SessionStatus};

fn save_command_session(context: &ActionContext, message: &str) -> Session {
    let transcript_path = context.temp_dir.path().join("transcript.jsonl");
    let record = json!({
        "type": "assistant",
        "message": {"content": [{"type": "text", "text": message}]},
    });
    std::fs::write(&transcript_path, record.to_string()).unwrap();
    let mut session = session("sample-session");
    session.engine = Engine::Codex;
    session.cwd = PathBuf::from(context.temp_dir.path());
    session.transcript_path = Some(transcript_path);
    context.save_session(&session);
    session
}

#[rstest]
#[tokio::test]
async fn command_preview_shows_cwd_only_for_a_transcript_command(action_context: ActionContext) {
    let transcript_path = action_context.temp_dir.path().join("transcript.jsonl");
    std::fs::write(
        &transcript_path,
        r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Run `! printf safe` now."}]}}"#,
    )
    .unwrap();
    let mut session = session("sample-session");
    session.engine = Engine::Codex;
    session.cwd = action_context.temp_dir.path().join("workspace");
    session.transcript_path = Some(transcript_path);
    std::fs::create_dir(&session.cwd).unwrap();
    action_context.save_session(&session);

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/commands/preview",
            json!({
                "messageText": "Run `! printf safe` now.",
                "commands": ["printf safe"],
            }),
        ))
        .await
        .unwrap();
    let body = response_snapshot(response, &action_context).await;

    assert_eq!(
        body,
        json!({
            "status": StatusCode::OK.as_u16(),
            "body": {"cwd": session.cwd.display().to_string()},
            "calls": [],
        }),
    );
}

#[rstest]
#[tokio::test]
async fn command_preview_accepts_indented_code_commands(action_context: ActionContext) {
    let message = "    ! printf safe";
    let session = save_command_session(&action_context, message);

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/commands/preview",
            json!({
                "messageText": message,
                "commands": ["printf safe"],
            }),
        ))
        .await
        .unwrap();
    let body = response_snapshot(response, &action_context).await;

    assert_eq!(
        body,
        json!({
            "status": StatusCode::OK.as_u16(),
            "body": {"cwd": session.cwd.display().to_string()},
            "calls": [],
        }),
    );
}

#[rstest]
#[tokio::test]
async fn command_execution_rejects_commands_absent_from_the_transcript(
    action_context: ActionContext,
) {
    let transcript_path = action_context.temp_dir.path().join("transcript.jsonl");
    std::fs::write(
        &transcript_path,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Run `! printf safe` now."}]}}"#,
    )
    .unwrap();
    let mut session = session("sample-session");
    session.engine = Engine::Codex;
    session.cwd = PathBuf::from(action_context.temp_dir.path());
    session.transcript_path = Some(transcript_path);
    action_context.save_session(&session);

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/commands/execute",
            json!({
                "messageText": "Run `! printf safe` now.",
                "commands": ["printf unsafe"],
            }),
        ))
        .await
        .unwrap();
    let body = response_snapshot(response, &action_context).await;

    assert_eq!(
        body,
        json!({
            "status": StatusCode::CONFLICT.as_u16(),
            "body": {"error": "the selected command is not present in the agent transcript"},
            "calls": [],
        }),
    );
}

#[rstest]
#[tokio::test]
async fn command_execution_rejects_ended_sessions(action_context: ActionContext) {
    let message = "Run `! printf safe` now.";
    let mut session = save_command_session(&action_context, message);
    session.status = SessionStatus::Ended;
    action_context.save_session(&session);

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/commands/execute",
            json!({
                "messageText": message,
                "commands": ["printf safe"],
            }),
        ))
        .await
        .unwrap();
    let body = response_snapshot(response, &action_context).await;

    assert_eq!(
        body,
        json!({
            "status": StatusCode::CONFLICT.as_u16(),
            "body": {"error": "session has ended"},
            "calls": [],
        }),
    );
}

#[rstest]
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn concurrent_identical_commands_execute_only_once(action_context: ActionContext) {
    let message = "Run `! printf safe` now.";
    save_command_session(&action_context, message);
    let router = action_context.router();
    let (command_started, release_command) = action_context.block_next_command();
    let first_router = router.clone();
    let first = tokio::spawn(async move {
        first_router
            .oneshot(post(
                "/sessions/sample-session/commands/execute",
                json!({
                    "messageText": message,
                    "commands": ["printf safe"],
                }),
            ))
            .await
            .unwrap()
    });
    command_started.await.unwrap();

    let second_router = router.clone();
    let second = tokio::spawn(async move {
        second_router
            .oneshot(post(
                "/sessions/sample-session/commands/execute",
                json!({
                    "messageText": message,
                    "commands": ["printf safe"],
                }),
            ))
            .await
            .unwrap()
    });
    let second = tokio::time::timeout(std::time::Duration::from_secs(1), second)
        .await
        .unwrap()
        .unwrap();
    let second = response_snapshot(second, &action_context).await;
    let _ = release_command.send(());
    let first = response_snapshot(first.await.unwrap(), &action_context).await;
    let cwd = action_context.temp_dir.path().display().to_string();
    let expected_message = indoc! {r#"
        Command exited with code 0 in {cwd}.

        Command:
        ```
        $ printf safe
        ```

        stdout:
        ```
        fixture output
        ```

        stderr:
        ```
        (empty)
        ```
    "#}
    .replace("{cwd}", &cwd);

    assert_eq!(
        (second, first),
        (
            json!({
                "status": StatusCode::CONFLICT.as_u16(),
                "body": {"error": "this command is already running"},
                "calls": [
                    json!({
                        "kind": "execute-command",
                        "cwd": action_context.temp_dir.path(),
                        "command": indoc! {r#"
                            set +e
                            __armyknife_bang_exit_code=0
                            printf safe
                            __armyknife_bang_command_exit_code=$?
                            if [ "$__armyknife_bang_command_exit_code" -ne 0 ] && [ "$__armyknife_bang_exit_code" -eq 0 ]; then __armyknife_bang_exit_code=$__armyknife_bang_command_exit_code; fi
                            exit "$__armyknife_bang_exit_code"
                        "#}
                        .trim_end(),
                    }),
                ],
            }),
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": {"exitCode": 0, "succeeded": true, "timedOut": false},
                "calls": [
                    json!({
                        "kind": "execute-command",
                        "cwd": action_context.temp_dir.path(),
                        "command": indoc! {r#"
                            set +e
                            __armyknife_bang_exit_code=0
                            printf safe
                            __armyknife_bang_command_exit_code=$?
                            if [ "$__armyknife_bang_command_exit_code" -ne 0 ] && [ "$__armyknife_bang_exit_code" -eq 0 ]; then __armyknife_bang_exit_code=$__armyknife_bang_command_exit_code; fi
                            exit "$__armyknife_bang_exit_code"
                        "#}
                        .trim_end(),
                    }),
                    json!({
                        "kind": "message",
                        "sessionId": "sample-session",
                        "content": expected_message.trim_end(),
                    }),
                ],
            }),
        ),
    );
}

#[rstest]
#[case::claude(Engine::Claude)]
#[case::codex(Engine::Codex)]
#[tokio::test]
async fn command_execution_returns_output_to_the_session(
    #[case] engine: Engine,
    action_context: ActionContext,
) {
    let transcript_path = action_context.temp_dir.path().join("transcript.jsonl");
    let message = indoc! {"
        ```sh
        ! printf safe
        ```
    "}
    .trim_end();
    let record = json!({
        "type": "assistant",
        "message": {"content": [{"type": "text", "text": message}]},
    });
    std::fs::write(&transcript_path, record.to_string()).unwrap();
    let mut session = session("sample-session");
    session.engine = engine;
    session.cwd = PathBuf::from(action_context.temp_dir.path());
    session.transcript_path = Some(transcript_path);
    action_context.save_session(&session);

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/commands/execute",
            json!({
                "messageText": message,
                "commands": ["printf safe"],
            }),
        ))
        .await
        .unwrap();
    let body = response_snapshot(response, &action_context).await;
    let has_marker = crate::commands::agent::store::watch_web_user_marker_from(
        &action_context.temp_dir.path().join("sessions"),
        "sample-session",
    )
    .unwrap()
    .is_some();
    let cwd = action_context.temp_dir.path().display().to_string();
    let expected_message = indoc! {r#"
        Command exited with code 0 in {cwd}.

        Command:
        ```
        $ printf safe
        ```

        stdout:
        ```
        fixture output
        ```

        stderr:
        ```
        (empty)
        ```
    "#}
    .replace("{cwd}", &cwd);

    assert_eq!(
        (body, has_marker),
        (
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": {"exitCode": 0, "succeeded": true, "timedOut": false},
                "calls": [
                    json!({
                        "kind": "execute-command",
                        "cwd": action_context.temp_dir.path(),
                        "command": indoc::indoc! {"
                            set +e
                            __armyknife_bang_exit_code=0
                            printf safe
                            __armyknife_bang_command_exit_code=$?
                            if [ \"$__armyknife_bang_command_exit_code\" -ne 0 ] && [ \"$__armyknife_bang_exit_code\" -eq 0 ]; then __armyknife_bang_exit_code=$__armyknife_bang_command_exit_code; fi
                            exit \"$__armyknife_bang_exit_code\"
                        "}
                        .trim_end(),
                    }),
                    json!({
                        "kind": "message",
                        "sessionId": "sample-session",
                        "content": expected_message.trim_end(),
                    }),
                ],
            }),
            engine == Engine::Claude,
        ),
    );
}

#[rstest]
#[tokio::test]
async fn command_execution_delivers_output_when_queue_storage_fails(action_context: ActionContext) {
    let message = "Run `! printf safe` now.";
    let session = save_command_session(&action_context, message);
    let database = rusqlite::Connection::open(
        action_context
            .temp_dir
            .path()
            .join("sessions")
            .join("sessions.db"),
    )
    .unwrap();
    database
        .execute_batch(
            "CREATE TRIGGER fail_watch_web_queued_prompt_insert
             BEFORE INSERT ON watch_web_queued_prompt
             BEGIN
                 SELECT RAISE(FAIL, 'queue unavailable');
             END;",
        )
        .unwrap();
    drop(database);

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/commands/execute",
            json!({
                "messageText": message,
                "commands": ["printf safe"],
            }),
        ))
        .await
        .unwrap();
    let body = response_snapshot(response, &action_context).await;
    let queued_prompts = normalized_queued_prompts(&action_context, "sample-session", None);
    let cwd = action_context.temp_dir.path().display().to_string();
    let expected_message = indoc! {r#"
        Command exited with code 0 in {cwd}.

        Command:
        ```
        $ printf safe
        ```

        stdout:
        ```
        fixture output
        ```

        stderr:
        ```
        (empty)
        ```
    "#}
    .replace("{cwd}", &cwd);

    assert_eq!(
        (body, queued_prompts),
        (
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": {"exitCode": 0, "succeeded": true, "timedOut": false},
                "calls": [
                    json!({
                        "kind": "execute-command",
                        "cwd": session.cwd,
                        "command": indoc! {r#"
                            set +e
                            __armyknife_bang_exit_code=0
                            printf safe
                            __armyknife_bang_command_exit_code=$?
                            if [ "$__armyknife_bang_command_exit_code" -ne 0 ] && [ "$__armyknife_bang_exit_code" -eq 0 ]; then __armyknife_bang_exit_code=$__armyknife_bang_command_exit_code; fi
                            exit "$__armyknife_bang_exit_code"
                        "#}
                        .trim_end(),
                    }),
                    json!({
                        "kind": "message",
                        "sessionId": "sample-session",
                        "content": expected_message.trim_end(),
                    }),
                ],
            }),
            json!([]),
        ),
    );
}

#[rstest]
#[tokio::test]
async fn timed_out_command_sends_partial_output_and_reports_timeout(action_context: ActionContext) {
    let transcript_path = action_context.temp_dir.path().join("transcript.jsonl");
    let message = "Run `! printf safe` now.";
    let record = json!({
        "type": "assistant",
        "message": {"content": [{"type": "text", "text": message}]},
    });
    std::fs::write(&transcript_path, record.to_string()).unwrap();
    let mut session = session("sample-session");
    session.engine = Engine::Codex;
    session.cwd = PathBuf::from(action_context.temp_dir.path());
    session.transcript_path = Some(transcript_path);
    action_context.save_session(&session);
    action_context.set_command_result(CommandResult {
        exit_code: 124,
        stdout: "partial output".to_string(),
        stderr: "command error".to_string(),
        timed_out: true,
    });

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/commands/execute",
            json!({
                "messageText": message,
                "commands": ["printf safe"],
            }),
        ))
        .await
        .unwrap();
    let body = response_snapshot(response, &action_context).await;
    let cwd = action_context.temp_dir.path().display().to_string();
    let expected_message = indoc! {r#"
        Command timed out after 30 seconds in {cwd}.

        Command:
        ```
        $ printf safe
        ```

        stdout:
        ```
        partial output
        ```

        stderr:
        ```
        command error
        ```
    "#}
    .replace("{cwd}", &cwd);

    assert_eq!(
        body,
        json!({
            "status": StatusCode::OK.as_u16(),
            "body": {"exitCode": 124, "succeeded": false, "timedOut": true},
            "calls": [
                json!({
                    "kind": "execute-command",
                    "cwd": action_context.temp_dir.path(),
                    "command": indoc! {r#"
                        set +e
                        __armyknife_bang_exit_code=0
                        printf safe
                        __armyknife_bang_command_exit_code=$?
                        if [ "$__armyknife_bang_command_exit_code" -ne 0 ] && [ "$__armyknife_bang_exit_code" -eq 0 ]; then __armyknife_bang_exit_code=$__armyknife_bang_command_exit_code; fi
                        exit "$__armyknife_bang_exit_code"
                    "#}
                    .trim_end(),
                }),
                json!({
                    "kind": "message",
                    "sessionId": "sample-session",
                    "content": expected_message.trim_end(),
                }),
            ],
        }),
    );
}
