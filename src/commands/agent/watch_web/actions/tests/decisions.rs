use axum::http::StatusCode;
use indoc::indoc;
use rstest::rstest;
use serde_json::json;
use tower::ServiceExt;

use super::{ActionContext, action_context, post, response_snapshot, session};
use crate::commands::agent::store;
use crate::commands::agent::types::SessionStatus;
use crate::commands::agent::watch_web_user_marker;

fn seed_decision(context: &ActionContext, session_id: &str) -> i64 {
    let sessions_dir = context.temp_dir.path().join("sessions");
    store::insert_agent_decision(
        &sessions_dir,
        session_id,
        "Choose the fixture path",
        "The fixture remains reversible.",
        &["Choose the other fixture path".to_string()],
        "Change one fixture value.",
    )
    .unwrap()
}

#[rstest]
#[tokio::test]
async fn ok_endpoint_marks_the_decision_reviewed(action_context: ActionContext) {
    let session = session("fixture-session");
    action_context.save_session(&session);
    let seq = seed_decision(&action_context, &session.session_id);

    let response = action_context
        .router()
        .oneshot(post(
            &format!("/sessions/{}/decisions/{seq}/ok", session.session_id),
            json!({}),
        ))
        .await
        .unwrap();
    let review = store::agent_decisions_from(
        &action_context.temp_dir.path().join("sessions"),
        &session.session_id,
    )
    .unwrap()[0]
        .review
        .clone();

    assert_eq!(
        (response_snapshot(response, &action_context).await, review),
        (
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": {"ok": true},
                "calls": [],
            }),
            Some("ok".to_string()),
        ),
    );
}

#[rstest]
#[tokio::test]
async fn ok_endpoint_returns_not_found_for_an_unknown_decision(action_context: ActionContext) {
    let session = session("fixture-session");
    action_context.save_session(&session);

    let response = action_context
        .router()
        .oneshot(post("/sessions/fixture-session/decisions/1/ok", json!({})))
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(response, &action_context).await,
        json!({
            "status": StatusCode::NOT_FOUND.as_u16(),
            "body": {"error": "decision not found"},
            "calls": [],
        }),
    );
}

#[rstest]
#[tokio::test]
async fn change_endpoint_rejects_an_empty_comment_without_side_effects(
    action_context: ActionContext,
) {
    let session = session("fixture-session");
    action_context.save_session(&session);
    let seq = seed_decision(&action_context, &session.session_id);

    let response = action_context
        .router()
        .oneshot(post(
            &format!("/sessions/fixture-session/decisions/{seq}/change"),
            json!({"comment": "  "}),
        ))
        .await
        .unwrap();
    let review = store::agent_decisions_from(
        &action_context.temp_dir.path().join("sessions"),
        &session.session_id,
    )
    .unwrap()[0]
        .review
        .clone();

    assert_eq!(
        (response_snapshot(response, &action_context).await, review),
        (
            json!({
                "status": StatusCode::BAD_REQUEST.as_u16(),
                "body": {"error": "comment must not be empty"},
                "calls": [],
            }),
            None,
        ),
    );
}

#[rstest]
#[tokio::test]
async fn change_endpoint_rejects_an_archived_parent(action_context: ActionContext) {
    let mut child = session("fixture-child-session");
    child.status = SessionStatus::Ended;
    child.ancestor_session_ids = vec!["fixture-parent-session".to_string()];
    let mut parent = session("fixture-parent-session");
    parent.archived_at = Some(parent.updated_at);
    action_context.save_session(&child);
    action_context.save_session(&parent);
    let seq = seed_decision(&action_context, &child.session_id);

    let response = action_context
        .router()
        .oneshot(post(
            &format!("/sessions/{}/decisions/{seq}/change", child.session_id),
            json!({"comment": "Use the fixture alternative."}),
        ))
        .await
        .unwrap();
    let sessions_dir = action_context.temp_dir.path().join("sessions");
    let review = store::agent_decisions_from(&sessions_dir, &child.session_id).unwrap()[0]
        .review
        .clone();

    assert_eq!(
        (response_snapshot(response, &action_context).await, review),
        (
            json!({
                "status": StatusCode::CONFLICT.as_u16(),
                "body": {"error": "parent session is archived"},
                "calls": [],
            }),
            None,
        ),
    );
}

#[rstest]
#[case::ended_child_targets_parent(SessionStatus::Ended, "fixture-parent-session", true)]
#[case::done_child_targets_parent(SessionStatus::Paused, "fixture-parent-session", true)]
#[case::running_child_targets_itself(SessionStatus::Running, "fixture-child-session", false)]
#[tokio::test]
async fn change_endpoint_notifies_the_selected_session_and_marks_the_decision(
    #[case] child_status: SessionStatus,
    #[case] target_id: &str,
    #[case] targets_parent: bool,
    action_context: ActionContext,
) {
    let mut child = session("fixture-child-session");
    child.status = child_status;
    child.ancestor_session_ids = vec!["fixture-parent-session".to_string()];
    let parent = session("fixture-parent-session");
    action_context.save_session(&child);
    action_context.save_session(&parent);
    let seq = seed_decision(&action_context, &child.session_id);

    let response = action_context
        .router()
        .oneshot(post(
            &format!("/sessions/{}/decisions/{seq}/change", child.session_id),
            json!({"comment": "Use the fixture alternative."}),
        ))
        .await
        .unwrap();
    let sessions_dir = action_context.temp_dir.path().join("sessions");
    let marker = store::watch_web_user_marker_from(&sessions_dir, target_id)
        .unwrap()
        .unwrap();
    let call = action_context.runtime.calls.lock().unwrap()[0].clone();
    let sent_content = call["content"].as_str().unwrap();
    let expected_content = if targets_parent {
        indoc! {"
            Decision from session fixture-child-session: Choose the fixture path

            Requested change: Use the fixture alternative.
        "}
    } else {
        indoc! {"
            Requested change to decision: Choose the fixture path

            Use the fixture alternative.
        "}
    };
    let marker_matches_target = watch_web_user_marker::strip_message(&marker, sent_content)
        .is_some_and(|content| content == expected_content);
    let review = store::agent_decisions_from(&sessions_dir, &child.session_id).unwrap()[0]
        .review
        .clone();

    assert_eq!(
        (
            response_snapshot(response, &action_context).await,
            marker_matches_target,
            review,
        ),
        (
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": {"ok": true},
                "calls": [{
                    "kind": "message",
                    "sessionId": target_id,
                    "content": expected_content,
                }],
            }),
            true,
            Some("change".to_string()),
        ),
    );
}
