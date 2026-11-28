use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use rstest::{fixture, rstest};
use serde_json::json;
use tokio::time::{Duration, advance};
use tower::ServiceExt;

use super::{ActionContext, action_context, post_empty, response_snapshot, session};
use crate::commands::agent::store;
use crate::commands::agent::types::{Engine, SessionStatus};

#[fixture]
fn rewind_action_context(
    #[default(SessionStatus::Ended)] successor_status: SessionStatus,
    action_context: ActionContext,
) -> ActionContext {
    let mut replaced = session("fixture-replaced");
    replaced.status = SessionStatus::Ended;
    let mut successor = session("fixture-successor");
    successor.status = successor_status;
    store::save_rewind_sessions(
        &action_context.temp_dir.path().join("sessions"),
        &replaced,
        &successor,
    )
    .unwrap();
    action_context
}

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
#[case::ended_pane_persistence_failed(SessionStatus::Ended)]
#[tokio::test(start_paused = true)]
async fn resume_succeeds_when_the_new_pane_id_cannot_be_persisted(
    #[case] status: SessionStatus,
    action_context: ActionContext,
) {
    let mut session = session("fixture-session");
    session.status = status;
    action_context.save_session(&session);
    rusqlite::Connection::open(action_context.temp_dir.path().join("sessions/sessions.db"))
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER reject_session_update BEFORE UPDATE OF data ON sessions
             BEGIN SELECT RAISE(ABORT, 'fixture session persistence failed'); END;",
        )
        .unwrap();
    let app = action_context.router();
    let mut updates = action_context.resume_updates.subscribe();

    let resume = app
        .clone()
        .oneshot(post_empty("/sessions/fixture-session/resume"))
        .await
        .unwrap();
    updates.changed().await.unwrap();
    let pending = app
        .oneshot(
            Request::builder()
                .uri("/sessions/fixture-session/actions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        (
            response_snapshot(resume, &action_context).await,
            response_snapshot(pending, &action_context).await,
            action_context
                .load_session("fixture-session")
                .tmux_info
                .map(|tmux_info| tmux_info.pane_id),
        ),
        (
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [{
                    "kind": "resume",
                    "sessionId": "fixture-session",
                    "status": status,
                }],
            }),
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": {
                    "message": false,
                    "answerQuestion": false,
                    "resume": true,
                    "startup": "pending",
                    "startupAt": "<startup-at>",
                    "interrupt": false,
                    "openInTerminal": true,
                    "rewind": false,
                },
                "calls": [{
                    "kind": "resume",
                    "sessionId": "fixture-session",
                    "status": status,
                }],
            }),
            Some("%4".to_string()),
        ),
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

#[rstest]
#[tokio::test]
async fn resume_endpoint_rejects_a_session_with_a_missing_working_directory(
    action_context: ActionContext,
) {
    let mut session = session("fixture-session");
    session.status = SessionStatus::Ended;
    session.cwd = action_context
        .temp_dir
        .path()
        .join("missing-working-directory");
    let working_directory = session.cwd.display().to_string();
    action_context.save_session(&session);

    let response = action_context
        .router()
        .oneshot(post_empty("/sessions/fixture-session/resume"))
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(response, &action_context).await,
        json!({
            "status": StatusCode::BAD_GATEWAY.as_u16(),
            "body": {
                "error": format!(
                    "session working directory does not exist: {working_directory}"
                ),
            },
            "calls": [],
        }),
    );
}

#[rstest]
#[tokio::test]
async fn resume_endpoint_rejects_a_replaced_id_when_its_successor_is_active(
    #[with(SessionStatus::Running)] rewind_action_context: ActionContext,
) {
    let response = rewind_action_context
        .router()
        .oneshot(post_empty("/sessions/fixture-replaced/resume"))
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(response, &rewind_action_context).await,
        json!({
            "status": StatusCode::CONFLICT.as_u16(),
            "body": { "error": "session is not ended or paused" },
            "calls": [],
        }),
    );
}

#[rstest]
#[tokio::test]
async fn actions_endpoint_uses_the_successor_status_for_a_replaced_id(
    #[with(SessionStatus::Running)] rewind_action_context: ActionContext,
) {
    let response = rewind_action_context
        .router()
        .oneshot(
            Request::builder()
                .uri("/sessions/fixture-replaced/actions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(response, &rewind_action_context).await,
        json!({
            "status": StatusCode::OK.as_u16(),
            "body": {
                "message": true,
                "answerQuestion": false,
                "resume": false,
                "interrupt": true,
                "openInTerminal": true,
                "rewind": false,
            },
            "calls": [],
        }),
    );
}

#[rstest]
#[tokio::test]
async fn repeated_resume_requests_for_a_replaced_id_share_the_successor_lock(
    #[with(SessionStatus::Ended)] rewind_action_context: ActionContext,
) {
    let app = rewind_action_context.router();

    let first = app
        .clone()
        .oneshot(post_empty("/sessions/fixture-replaced/resume"))
        .await
        .unwrap();
    let second = app
        .oneshot(post_empty("/sessions/fixture-replaced/resume"))
        .await
        .unwrap();

    let calls = json!([{
        "kind": "resume",
        "sessionId": "fixture-successor",
        "status": SessionStatus::Ended,
    }]);
    assert_eq!(
        (
            response_snapshot(first, &rewind_action_context).await,
            response_snapshot(second, &rewind_action_context).await,
        ),
        (
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": calls.clone(),
            }),
            json!({
                "status": StatusCode::CONFLICT.as_u16(),
                "body": { "error": "session resume is already in progress" },
                "calls": calls,
            }),
        ),
    );
}

#[rstest]
#[tokio::test]
async fn resume_endpoint_rejects_a_second_request_until_the_session_starts(
    action_context: ActionContext,
) {
    let mut session = session("fixture-session");
    session.status = SessionStatus::Ended;
    action_context.save_session(&session);
    let app = action_context.router();

    let first = app
        .clone()
        .oneshot(post_empty("/sessions/fixture-session/resume"))
        .await
        .unwrap();
    let second = app
        .oneshot(post_empty("/sessions/fixture-session/resume"))
        .await
        .unwrap();

    let calls = json!([{
        "kind": "resume",
        "sessionId": "fixture-session",
        "status": SessionStatus::Ended,
    }]);
    assert_eq!(
        (
            response_snapshot(first, &action_context).await,
            response_snapshot(second, &action_context).await,
        ),
        (
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": calls.clone(),
            }),
            json!({
                "status": StatusCode::CONFLICT.as_u16(),
                "body": { "error": "session resume is already in progress" },
                "calls": calls,
            }),
        ),
    );
}

#[rstest]
#[tokio::test(start_paused = true)]
async fn actions_report_stuck_resume_after_twenty_seconds_and_clear_it_when_started(
    action_context: ActionContext,
) {
    let mut session = session("fixture-session");
    session.status = SessionStatus::Ended;
    action_context.save_session(&session);
    let app = action_context.router();
    let mut updates = action_context.resume_updates.subscribe();

    let resume = app
        .clone()
        .oneshot(post_empty("/sessions/fixture-session/resume"))
        .await
        .unwrap();
    updates.changed().await.unwrap();

    let pending = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/sessions/fixture-session/actions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    advance(Duration::from_secs(20)).await;
    updates.changed().await.unwrap();

    let stuck = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/sessions/fixture-session/actions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    session.status = SessionStatus::Running;
    action_context.save_session(&session);
    let started = app
        .oneshot(
            Request::builder()
                .uri("/sessions/fixture-session/actions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        (
            response_snapshot(resume, &action_context).await,
            response_snapshot(pending, &action_context).await,
            response_snapshot(stuck, &action_context).await,
            response_snapshot(started, &action_context).await,
        ),
        (
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [{
                    "kind": "resume",
                    "sessionId": "fixture-session",
                    "status": SessionStatus::Ended,
                }],
            }),
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": {
                    "message": false,
                    "answerQuestion": false,
                    "resume": true,
                    "startup": "pending",
                    "startupAt": "<startup-at>",
                    "interrupt": false,
                    "openInTerminal": true,
                    "rewind": false,
                },
                "calls": [{
                    "kind": "resume",
                    "sessionId": "fixture-session",
                    "status": SessionStatus::Ended,
                }],
            }),
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": {
                    "message": false,
                    "answerQuestion": false,
                    "resume": true,
                    "startup": "stuck",
                    "startupAt": "<startup-at>",
                    "interrupt": false,
                    "openInTerminal": true,
                    "rewind": false,
                },
                "calls": [{
                    "kind": "resume",
                    "sessionId": "fixture-session",
                    "status": SessionStatus::Ended,
                }],
            }),
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": {
                    "message": true,
                    "answerQuestion": false,
                    "resume": false,
                    "interrupt": true,
                    "openInTerminal": true,
                    "rewind": false,
                },
                "calls": [{
                    "kind": "resume",
                    "sessionId": "fixture-session",
                    "status": SessionStatus::Ended,
                }],
            }),
        ),
    );
}

#[rstest]
#[tokio::test(start_paused = true)]
async fn retrying_a_stuck_resume_restarts_the_startup_timeout(action_context: ActionContext) {
    let mut session = session("fixture-session");
    session.status = SessionStatus::Ended;
    action_context.save_session(&session);
    let app = action_context.router();
    let mut updates = action_context.resume_updates.subscribe();

    let resume = app
        .clone()
        .oneshot(post_empty("/sessions/fixture-session/resume"))
        .await
        .unwrap();
    updates.changed().await.unwrap();
    advance(Duration::from_secs(20)).await;
    updates.changed().await.unwrap();

    let retry = app
        .clone()
        .oneshot(post_empty("/sessions/fixture-session/resume/retry"))
        .await
        .unwrap();
    updates.changed().await.unwrap();
    let actions = app
        .oneshot(
            Request::builder()
                .uri("/sessions/fixture-session/actions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        (
            response_snapshot(resume, &action_context).await,
            response_snapshot(retry, &action_context).await,
            response_snapshot(actions, &action_context).await,
        ),
        (
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [
                    {
                        "kind": "resume",
                        "sessionId": "fixture-session",
                        "status": SessionStatus::Ended,
                    },
                    {
                        "kind": "retry-resume",
                        "sessionId": "fixture-session",
                        "status": SessionStatus::Ended,
                    },
                ],
            }),
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [
                    {
                        "kind": "resume",
                        "sessionId": "fixture-session",
                        "status": SessionStatus::Ended,
                    },
                    {
                        "kind": "retry-resume",
                        "sessionId": "fixture-session",
                        "status": SessionStatus::Ended,
                    },
                ],
            }),
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": {
                    "message": false,
                    "answerQuestion": false,
                    "resume": true,
                    "startup": "pending",
                    "startupAt": "<startup-at>",
                    "interrupt": false,
                    "openInTerminal": true,
                    "rewind": false,
                },
                "calls": [
                    {
                        "kind": "resume",
                        "sessionId": "fixture-session",
                        "status": SessionStatus::Ended,
                    },
                    {
                        "kind": "retry-resume",
                        "sessionId": "fixture-session",
                        "status": SessionStatus::Ended,
                    },
                ],
            }),
        ),
    );
}

#[rstest]
#[case::never_resumed(false, false)]
#[case::still_pending(true, false)]
#[case::started_after_stuck(true, true)]
#[tokio::test(start_paused = true)]
async fn retry_resume_rejects_a_session_that_is_not_stuck(
    #[case] start_resume: bool,
    #[case] session_started: bool,
    action_context: ActionContext,
) {
    let mut session = session("fixture-session");
    session.status = SessionStatus::Ended;
    action_context.save_session(&session);
    let app = action_context.router();
    let mut updates = action_context.resume_updates.subscribe();

    if start_resume {
        app.clone()
            .oneshot(post_empty("/sessions/fixture-session/resume"))
            .await
            .unwrap();
        updates.changed().await.unwrap();
    }
    if session_started {
        advance(Duration::from_secs(20)).await;
        updates.changed().await.unwrap();
        session.status = SessionStatus::Running;
        session.updated_at += chrono::Duration::seconds(1);
        action_context.record_session_start(&session);
    }
    let retry = app
        .oneshot(post_empty("/sessions/fixture-session/resume/retry"))
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(retry, &action_context).await,
        json!({
            "status": StatusCode::CONFLICT.as_u16(),
            "body": { "error": "session startup is not stuck" },
            "calls": if start_resume {
                json!([{
                    "kind": "resume",
                    "sessionId": "fixture-session",
                    "status": SessionStatus::Ended,
                }])
            } else {
                json!([])
            },
        }),
    );
}

#[rstest]
#[tokio::test(start_paused = true)]
async fn codex_resume_keeps_startup_status_while_marked_stopped(action_context: ActionContext) {
    let mut session = session("fixture-session");
    session.engine = Engine::Codex;
    session.status = SessionStatus::Ended;
    action_context.save_session(&session);
    let app = action_context.router();
    let mut updates = action_context.resume_updates.subscribe();

    let resume = app
        .clone()
        .oneshot(post_empty("/sessions/fixture-session/resume"))
        .await
        .unwrap();
    updates.changed().await.unwrap();

    session.status = SessionStatus::Stopped;
    session.updated_at += chrono::Duration::seconds(1);
    action_context.save_session(&session);

    let pending = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/sessions/fixture-session/actions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let duplicate_resume = app
        .clone()
        .oneshot(post_empty("/sessions/fixture-session/resume"))
        .await
        .unwrap();

    advance(Duration::from_secs(20)).await;
    updates.changed().await.unwrap();

    let retry_after_stuck = app
        .clone()
        .oneshot(post_empty("/sessions/fixture-session/resume"))
        .await
        .unwrap();
    let stuck = app
        .oneshot(
            Request::builder()
                .uri("/sessions/fixture-session/actions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        (
            response_snapshot(resume, &action_context).await,
            response_snapshot(pending, &action_context).await,
            response_snapshot(duplicate_resume, &action_context).await,
            response_snapshot(retry_after_stuck, &action_context).await,
            response_snapshot(stuck, &action_context).await,
        ),
        (
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [{
                    "kind": "resume",
                    "sessionId": "fixture-session",
                    "status": SessionStatus::Ended,
                }],
            }),
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": {
                    "message": true,
                    "answerQuestion": false,
                    "resume": false,
                    "startup": "pending",
                    "startupAt": "<startup-at>",
                    "interrupt": false,
                    "openInTerminal": true,
                    "rewind": true,
                },
                "calls": [{
                    "kind": "resume",
                    "sessionId": "fixture-session",
                    "status": SessionStatus::Ended,
                }],
            }),
            json!({
                "status": StatusCode::CONFLICT.as_u16(),
                "body": { "error": "session resume is already in progress" },
                "calls": [{
                    "kind": "resume",
                    "sessionId": "fixture-session",
                    "status": SessionStatus::Ended,
                }],
            }),
            json!({
                "status": StatusCode::CONFLICT.as_u16(),
                "body": { "error": "session is not ended or paused" },
                "calls": [{
                    "kind": "resume",
                    "sessionId": "fixture-session",
                    "status": SessionStatus::Ended,
                }],
            }),
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": {
                    "message": true,
                    "answerQuestion": false,
                    "resume": false,
                    "startup": "stuck",
                    "startupAt": "<startup-at>",
                    "interrupt": false,
                    "openInTerminal": true,
                    "rewind": true,
                },
                "calls": [{
                    "kind": "resume",
                    "sessionId": "fixture-session",
                    "status": SessionStatus::Ended,
                }],
            }),
        ),
    );
}

#[rstest]
#[case::started_before_deadline(SessionStatus::Ended, true, None)]
#[case::unrelated_update_while_paused(SessionStatus::Paused, false, Some("stuck"))]
#[tokio::test(start_paused = true)]
async fn startup_status_uses_the_session_start_marker(
    #[case] initial_status: SessionStatus,
    #[case] started_before_deadline: bool,
    #[case] expected_startup: Option<&str>,
    action_context: ActionContext,
) {
    let mut session = session("fixture-session");
    session.status = initial_status;
    action_context.save_session(&session);
    let app = action_context.router();
    let mut updates = action_context.resume_updates.subscribe();

    app.clone()
        .oneshot(post_empty("/sessions/fixture-session/resume"))
        .await
        .unwrap();
    updates.changed().await.unwrap();

    advance(Duration::from_secs(10)).await;
    if started_before_deadline {
        session.status = SessionStatus::Running;
        session.updated_at += chrono::Duration::seconds(1);
        action_context.record_session_start(&session);
    } else {
        session.updated_at += chrono::Duration::seconds(1);
        action_context.save_session(&session);
    }

    advance(Duration::from_secs(10)).await;
    updates.changed().await.unwrap();

    if started_before_deadline {
        session.status = SessionStatus::Paused;
        session.updated_at += chrono::Duration::seconds(1);
        action_context.save_session(&session);
    }

    let actions = app
        .oneshot(
            Request::builder()
                .uri("/sessions/fixture-session/actions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let expected_body = match expected_startup {
        Some(startup) => json!({
            "message": true,
            "answerQuestion": false,
            "resume": true,
            "startup": startup,
            "startupAt": "<startup-at>",
            "interrupt": false,
            "openInTerminal": true,
            "rewind": true,
        }),
        None => json!({
            "message": true,
            "answerQuestion": false,
            "resume": true,
            "interrupt": false,
            "openInTerminal": true,
            "rewind": true,
        }),
    };

    assert_eq!(
        response_snapshot(actions, &action_context).await,
        json!({
            "status": StatusCode::OK.as_u16(),
            "body": expected_body,
            "calls": [{
                "kind": "resume",
                "sessionId": "fixture-session",
                "status": initial_status,
            }],
        }),
    );
}

#[rstest]
#[tokio::test(start_paused = true)]
async fn stale_resume_timeout_does_not_mark_a_new_attempt_stuck(action_context: ActionContext) {
    let mut session = session("fixture-session");
    session.status = SessionStatus::Ended;
    action_context.save_session(&session);
    let app = action_context.router();
    let mut updates = action_context.resume_updates.subscribe();

    app.clone()
        .oneshot(post_empty("/sessions/fixture-session/resume"))
        .await
        .unwrap();
    updates.changed().await.unwrap();
    advance(Duration::from_secs(10)).await;

    session.status = SessionStatus::Running;
    action_context.save_session(&session);
    app.clone()
        .oneshot(
            Request::builder()
                .uri("/sessions/fixture-session/actions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    session.status = SessionStatus::Ended;
    action_context.save_session(&session);
    app.clone()
        .oneshot(post_empty("/sessions/fixture-session/resume"))
        .await
        .unwrap();
    updates.changed().await.unwrap();

    advance(Duration::from_secs(10)).await;
    tokio::task::yield_now().await;

    let actions = app
        .oneshot(
            Request::builder()
                .uri("/sessions/fixture-session/actions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(actions, &action_context).await,
        json!({
            "status": StatusCode::OK.as_u16(),
            "body": {
                "message": false,
                "answerQuestion": false,
                "resume": true,
                "startup": "pending",
                "startupAt": "<startup-at>",
                "interrupt": false,
                "openInTerminal": true,
                "rewind": false,
            },
            "calls": [
                {
                    "kind": "resume",
                    "sessionId": "fixture-session",
                    "status": SessionStatus::Ended,
                },
                {
                    "kind": "resume",
                    "sessionId": "fixture-session",
                    "status": SessionStatus::Ended,
                },
            ],
        }),
    );
}

#[rstest]
#[tokio::test]
async fn ended_session_without_tmux_location_cannot_be_resumed(action_context: ActionContext) {
    let mut session = session("fixture-session");
    session.status = SessionStatus::Ended;
    session.tmux_info = None;
    action_context.save_session(&session);

    let response = action_context
        .router()
        .oneshot(
            Request::builder()
                .uri("/sessions/fixture-session/actions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(response, &action_context).await,
        json!({
            "status": StatusCode::OK.as_u16(),
            "body": {
                "message": false,
                "answerQuestion": false,
                "resume": false,
                "resumeDisabledReason": "noTmux",
                "interrupt": false,
                "openInTerminal": false,
                "rewind": false,
            },
            "calls": [],
        }),
    );
}
