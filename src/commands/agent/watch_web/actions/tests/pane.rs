use axum::body::Body;
use axum::http::{Request, StatusCode};
use rstest::rstest;
use serde_json::json;
use tower::ServiceExt;

use super::{ActionContext, action_context, response_snapshot, session};

#[rstest]
#[tokio::test]
async fn pane_endpoint_returns_the_ansi_styled_capture(action_context: ActionContext) {
    action_context.save_session(&session("fixture-session"));
    action_context.set_pane_snapshot("\u{1b}[31mfixture output\u{1b}[0m");

    let response = action_context
        .router()
        .oneshot(get("/sessions/fixture-session/pane"))
        .await
        .unwrap();

    assert_eq!(
        response_snapshot(response, &action_context).await,
        json!({
            "status": StatusCode::OK.as_u16(),
            "body": { "content": "\u{1b}[31mfixture output\u{1b}[0m" },
            "calls": [{
                "kind": "pane-snapshot",
                "paneId": "%4",
                "socketPath": "/tmp/example-tmux.sock",
            }],
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
