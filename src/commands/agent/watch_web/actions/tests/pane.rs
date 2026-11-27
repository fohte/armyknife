use axum::body::Body;
use axum::http::{Request, StatusCode};
use rstest::rstest;
use serde_json::json;
use tower::ServiceExt;

use super::{ActionContext, action_context, response_snapshot, session};

#[rstest]
#[tokio::test]
async fn pane_endpoint_returns_the_plain_capture_and_timestamp(action_context: ActionContext) {
    action_context.save_session(&session("fixture-session"));
    let snapshot = ["", "", "fixture output", ""].join("\n");
    action_context.set_pane_snapshot(&snapshot);

    let response = action_context
        .router()
        .oneshot(get("/sessions/fixture-session/pane"))
        .await
        .unwrap();

    let mut actual = response_snapshot(response, &action_context).await;
    normalize_captured_at(&mut actual);

    assert_eq!(
        actual,
        json!({
            "status": StatusCode::OK.as_u16(),
            "body": {
                "content": snapshot,
                "capturedAt": "2000-01-02T03:04:05Z",
            },
            "calls": [{
                "kind": "pane-snapshot",
                "sessionId": "fixture-session",
                "paneId": "%4",
                "socketPath": "/tmp/example-tmux.sock",
            }],
        }),
    );
}

#[rstest]
#[tokio::test]
async fn pane_endpoint_returns_not_found_for_a_missing_session(action_context: ActionContext) {
    let response = action_context
        .router()
        .oneshot(get("/sessions/missing-session/pane"))
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

#[rstest]
#[tokio::test]
async fn pane_endpoint_reports_when_a_session_has_no_tmux_pane(action_context: ActionContext) {
    let mut session = session("fixture-session");
    session.tmux_info = None;
    action_context.save_session(&session);

    let response = action_context
        .router()
        .oneshot(get("/sessions/fixture-session/pane"))
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(response, &action_context).await,
        json!({
            "status": StatusCode::CONFLICT.as_u16(),
            "body": { "error": "session has no tmux pane" },
            "calls": [],
        }),
    );
}

#[rstest]
#[case::tmux_server_unavailable("fixture tmux server unavailable")]
#[case::pane_missing("fixture pane missing")]
#[tokio::test]
async fn pane_endpoint_returns_tmux_capture_errors(
    #[case] error: &str,
    action_context: ActionContext,
) {
    action_context.save_session(&session("fixture-session"));
    action_context.set_pane_snapshot_error(error);

    let response = action_context
        .router()
        .oneshot(get("/sessions/fixture-session/pane"))
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(response, &action_context).await,
        json!({
            "status": StatusCode::BAD_GATEWAY.as_u16(),
            "body": { "error": error },
            "calls": [{
                "kind": "pane-snapshot",
                "sessionId": "fixture-session",
                "paneId": "%4",
                "socketPath": "/tmp/example-tmux.sock",
            }],
        }),
    );
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

fn normalize_captured_at(snapshot: &mut serde_json::Value) {
    let captured_at = snapshot["body"]["capturedAt"]
        .as_str()
        .expect("capturedAt is a string");
    chrono::DateTime::parse_from_rfc3339(captured_at).expect("capturedAt uses RFC 3339");
    snapshot["body"]["capturedAt"] = json!("2000-01-02T03:04:05Z");
}
