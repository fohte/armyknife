use axum::http::StatusCode;
use rstest::rstest;
use serde_json::json;
use tower::ServiceExt;

use super::{ActionContext, action_context, post, response_snapshot, session};

#[rstest]
#[case::sets_trimmed_title("  A clearer title  ", Some("A clearer title"))]
#[case::clears_title_with_whitespace("   ", None)]
#[tokio::test]
async fn title_endpoint_saves_the_normalized_label(
    #[case] title: &str,
    #[case] expected_label: Option<&str>,
    action_context: ActionContext,
) {
    let mut session = session("fixture-session");
    session.label = Some("Old title".to_string());
    session.tmux_info = None;
    action_context.save_session(&session);

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/fixture-session/title",
            json!({ "title": title }),
        ))
        .await
        .unwrap();
    let saved_label = action_context.load_session("fixture-session").label;

    assert_eq!(
        (
            response_snapshot(response, &action_context).await,
            saved_label,
        ),
        (
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true },
                "calls": [],
            }),
            expected_label.map(str::to_string),
        ),
    );
}

#[rstest]
#[tokio::test]
async fn title_endpoint_rejects_unknown_sessions(action_context: ActionContext) {
    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/unknown-session/title",
            json!({ "title": "Fixture title" }),
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
