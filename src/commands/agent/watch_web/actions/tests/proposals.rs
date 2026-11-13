use axum::http::StatusCode;
use indoc::indoc;
use rstest::rstest;
use serde_json::json;
use tower::ServiceExt;

use super::{ActionContext, action_context, post, response_snapshot, session};
use crate::commands::agent::store;
use crate::commands::agent::types::{Engine, SessionStatus};

fn add_proposal(context: &ActionContext, session_id: &str) {
    store::add_agent_proposal_in(
        &context.temp_dir.path().join("sessions"),
        session_id,
        r#"{"title":"Pick a format","summary":null,"options":[{"label":"Format A","description":null,"pros":[],"cons":[],"recommended":false},{"label":"Format B","description":null,"pros":[],"cons":[],"recommended":false}]}"#,
    )
    .unwrap();
}

#[rstest]
#[tokio::test]
async fn chosen_option_is_notified_before_the_answer_is_stored(action_context: ActionContext) {
    action_context.save_session(&session("sample-session"));
    add_proposal(&action_context, "sample-session");

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/proposals/1/answer",
            json!({ "choice": 0, "comment": "Keep the simpler format." }),
        ))
        .await
        .unwrap();
    let proposals = store::agent_proposals_from(
        &action_context.temp_dir.path().join("sessions"),
        "sample-session",
    )
    .unwrap();

    assert_eq!(
        (
            response_snapshot(response, &action_context).await,
            proposals[0].answer.as_deref(),
            proposals[0].resolved_at.is_some(),
        ),
        (
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [{
                    "kind": "message",
                    "sessionId": "sample-session",
                    "content": indoc! {"
                        Proposal 1 \"Pick a format\": chose A (Format A).

                        Keep the simpler format.
                    "}.trim_end(),
                }],
            }),
            Some(r#"{"choice":0,"comment":"Keep the simpler format."}"#),
            true,
        ),
    );
}

#[rstest]
#[tokio::test]
async fn consecutive_proposals_remain_individually_answerable(action_context: ActionContext) {
    action_context.save_session(&session("sample-session"));
    add_proposal(&action_context, "sample-session");
    add_proposal(&action_context, "sample-session");

    let first_response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/proposals/1/answer",
            json!({ "choice": 0, "comment": null }),
        ))
        .await
        .unwrap();
    let first_snapshot = response_snapshot(first_response, &action_context).await;
    let second_response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/proposals/2/answer",
            json!({ "choice": 1, "comment": null }),
        ))
        .await
        .unwrap();
    let second_snapshot = response_snapshot(second_response, &action_context).await;
    let proposals = store::agent_proposals_from(
        &action_context.temp_dir.path().join("sessions"),
        "sample-session",
    )
    .unwrap();

    assert_eq!(
        (
            first_snapshot,
            second_snapshot,
            proposals
                .iter()
                .map(|proposal| (proposal.answer.clone(), proposal.resolved_at.is_some()))
                .collect::<Vec<_>>(),
        ),
        (
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [{
                    "kind": "message",
                    "sessionId": "sample-session",
                    "content": "Proposal 1 \"Pick a format\": chose A (Format A).",
                }],
            }),
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [
                    {
                        "kind": "message",
                        "sessionId": "sample-session",
                        "content": "Proposal 1 \"Pick a format\": chose A (Format A).",
                    },
                    {
                        "kind": "message",
                        "sessionId": "sample-session",
                        "content": "Proposal 2 \"Pick a format\": chose B (Format B).",
                    },
                ],
            }),
            vec![
                (Some(r#"{"choice":0,"comment":null}"#.to_string()), true),
                (Some(r#"{"choice":1,"comment":null}"#.to_string()), true),
            ],
        ),
    );
}

#[rstest]
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn concurrent_answers_notify_the_agent_only_once(action_context: ActionContext) {
    action_context.save_session(&session("sample-session"));
    add_proposal(&action_context, "sample-session");
    let (router, answer_locks) = action_context.router_with_proposal_answer_locks();
    let (message_started, release_message) = action_context.block_next_message();
    let first_router = router.clone();
    let first = tokio::spawn(async move {
        first_router
            .oneshot(post(
                "/sessions/sample-session/proposals/1/answer",
                json!({ "choice": 0, "comment": null }),
            ))
            .await
            .unwrap()
    });
    message_started.await.unwrap();

    let answer_lock = answer_locks.get_or_create("sample-session", 1).await;
    let second = tokio::spawn(async move {
        router
            .oneshot(post(
                "/sessions/sample-session/proposals/1/answer",
                json!({ "choice": 0, "comment": null }),
            ))
            .await
            .unwrap()
    });
    let second_waited_for_first = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while std::sync::Arc::strong_count(&answer_lock) < 3 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .is_ok();
    let sent_before_first_completed = action_context.runtime.calls.lock().unwrap().len();
    let _ = release_message.send(());

    let first = response_snapshot(first.await.unwrap(), &action_context).await;
    let second = response_snapshot(second.await.unwrap(), &action_context).await;
    let proposals = store::agent_proposals_from(
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
            proposals[0].answer.clone(),
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
                    "content": "Proposal 1 \"Pick a format\": chose A (Format A).",
                }],
            }),
            json!({
                "status": StatusCode::CONFLICT.as_u16(),
                "body": { "error": "proposal is not awaiting an answer" },
                "calls": [{
                    "kind": "message",
                    "sessionId": "sample-session",
                    "content": "Proposal 1 \"Pick a format\": chose A (Format A).",
                }],
            }),
            Some(r#"{"choice":0,"comment":null}"#.to_string()),
        ),
    );
}

#[rstest]
#[case::claude(Engine::Claude, true)]
#[case::codex(Engine::Codex, false)]
#[tokio::test]
async fn proposal_answers_use_a_marker_only_for_claude(
    #[case] engine: Engine,
    #[case] expects_marker: bool,
    action_context: ActionContext,
) {
    let mut session = session("sample-session");
    session.engine = engine;
    action_context.save_session(&session);
    add_proposal(&action_context, "sample-session");

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/proposals/1/answer",
            json!({ "choice": null, "comment": null }),
        ))
        .await
        .unwrap();
    let sessions_dir = action_context.temp_dir.path().join("sessions");
    let marker = store::watch_web_user_marker_from(&sessions_dir, "sample-session").unwrap();
    let was_marked = action_context.runtime.calls.lock().unwrap()[0]["content"]
        .as_str()
        .is_some_and(|content| {
            marker.as_deref().is_some_and(|marker| {
                crate::commands::agent::watch_web_user_marker::strip_message(marker, content)
                    .is_some()
            })
        });

    assert_eq!(
        (
            response_snapshot(response, &action_context).await,
            marker.is_some(),
            was_marked,
        ),
        (
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [{
                    "kind": "message",
                    "sessionId": "sample-session",
                    "content": "Proposal 1 \"Pick a format\": none of the options fit. Propose another option.",
                }],
            }),
            expects_marker,
            expects_marker,
        ),
    );
}

#[rstest]
#[case::ended_session(0, None, true, StatusCode::CONFLICT, "session has ended", None)]
#[case::answered_proposal(
    1,
    Some(r#"{"choice":0,"comment":null}"#),
    false,
    StatusCode::CONFLICT,
    "proposal is not awaiting an answer",
    Some(r#"{"choice":0,"comment":null}"#)
)]
#[case::choice_outside_options(
    2,
    None,
    false,
    StatusCode::BAD_REQUEST,
    "choice is outside the proposal options",
    None
)]
#[tokio::test]
async fn unavailable_or_invalid_proposal_answers_are_rejected(
    #[case] choice: usize,
    #[case] initial_answer: Option<&str>,
    #[case] session_ended: bool,
    #[case] expected_status: StatusCode,
    #[case] expected_error: &str,
    #[case] expected_answer: Option<&str>,
    action_context: ActionContext,
) {
    let mut session = session("sample-session");
    if session_ended {
        session.status = SessionStatus::Ended;
    }
    action_context.save_session(&session);
    add_proposal(&action_context, "sample-session");
    let sessions_dir = action_context.temp_dir.path().join("sessions");
    if let Some(answer) = initial_answer {
        store::write_agent_proposal_answer_from(&sessions_dir, "sample-session", 1, answer)
            .unwrap();
    }

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/proposals/1/answer",
            json!({ "choice": choice, "comment": null }),
        ))
        .await
        .unwrap();
    let proposals = store::agent_proposals_from(&sessions_dir, "sample-session").unwrap();

    assert_eq!(
        (
            response_snapshot(response, &action_context).await,
            proposals[0].answer.clone(),
        ),
        (
            json!({
                "status": expected_status.as_u16(),
                "body": { "error": expected_error },
                "calls": [],
            }),
            expected_answer.map(str::to_string),
        ),
    );
}

#[rstest]
#[tokio::test]
async fn failed_peer_notification_does_not_store_the_answer(action_context: ActionContext) {
    action_context.save_session(&session("sample-session"));
    add_proposal(&action_context, "sample-session");
    action_context.set_message_error("peer notify failed");

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/proposals/1/answer",
            json!({ "choice": 0, "comment": null }),
        ))
        .await
        .unwrap();
    let proposals = store::agent_proposals_from(
        &action_context.temp_dir.path().join("sessions"),
        "sample-session",
    )
    .unwrap();

    assert_eq!(
        (
            response_snapshot(response, &action_context).await,
            proposals[0].answer.clone(),
        ),
        (
            json!({
                "status": StatusCode::BAD_GATEWAY.as_u16(),
                "body": { "error": "peer notify failed" },
                "calls": [{
                    "kind": "message",
                    "sessionId": "sample-session",
                    "content": "Proposal 1 \"Pick a format\": chose A (Format A).",
                }],
            }),
            None,
        ),
    );
}

#[rstest]
#[tokio::test]
async fn save_failure_after_peer_notification_reports_the_partial_send(
    action_context: ActionContext,
) {
    action_context.save_session(&session("sample-session"));
    add_proposal(&action_context, "sample-session");
    let sessions_dir = action_context.temp_dir.path().join("sessions");
    action_context.set_after_message(move || {
        let database = sessions_dir.join("sessions.db");
        let backup = sessions_dir.join("sessions.db-backup");
        std::fs::rename(database, backup).unwrap();
        std::fs::create_dir(sessions_dir.join("sessions.db")).unwrap();
    });

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/proposals/1/answer",
            json!({ "choice": 0, "comment": null }),
        ))
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(response, &action_context).await,
        json!({
            "status": StatusCode::BAD_GATEWAY.as_u16(),
            "body": {
                "error": "proposal answer was sent but could not be saved; check the conversation before retrying",
            },
            "calls": [{
                "kind": "message",
                "sessionId": "sample-session",
                "content": "Proposal 1 \"Pick a format\": chose A (Format A).",
            }],
        }),
    );
}
