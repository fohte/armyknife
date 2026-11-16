use axum::http::StatusCode;
use indoc::indoc;
use rstest::rstest;
use serde_json::json;
use tower::ServiceExt;

use super::{
    ActionContext, action_context, normalized_queued_prompts, post, response_snapshot, session,
};
use crate::commands::agent::store;
use crate::shared::config::Config;

fn allowed_config() -> Config {
    serde_yaml::from_str(indoc! {"
        orgs:
          fixture-owner:
            vibe: true
    "})
    .unwrap()
}

#[rstest]
#[tokio::test]
async fn enabling_vibe_updates_session_and_sends_a_queued_prompt(action_context: ActionContext) {
    action_context.save_session(&session("fixture-session"));

    let response = action_context
        .router_with_vibe_config(allowed_config())
        .oneshot(post(
            "/sessions/fixture-session/vibe",
            json!({ "enabled": true }),
        ))
        .await
        .unwrap();
    let actual = (
        response_snapshot(response, &action_context).await,
        action_context.load_session("fixture-session").vibe_enabled,
        normalized_queued_prompts(&action_context, "fixture-session", None),
    );

    assert_eq!(
        actual,
        (
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true, "queuedPromptId": "<message-id>" },
                "calls": [{
                    "kind": "message",
                    "sessionId": "fixture-session",
                    "content": "watch-web からユーザーが vibe mode を on にしました。",
                }],
            }),
            true,
            json!([{
                "id": "<prompt-id>",
                "content": "watch-web からユーザーが vibe mode を on にしました。",
                "sentAt": "<sent-at>",
            }]),
        ),
    );
}

#[rstest]
#[tokio::test]
async fn enabling_vibe_is_rejected_when_repo_policy_disallows_it(action_context: ActionContext) {
    action_context.save_session(&session("fixture-session"));

    let response = action_context
        .router_with_vibe_config(Config::default())
        .oneshot(post(
            "/sessions/fixture-session/vibe",
            json!({ "enabled": true }),
        ))
        .await
        .unwrap();
    let actual = (
        response_snapshot(response, &action_context).await,
        action_context.load_session("fixture-session").vibe_enabled,
        store::watch_web_queued_prompts_from(
            &action_context.temp_dir.path().join("sessions"),
            "fixture-session",
        )
        .unwrap(),
    );

    assert_eq!(
        actual,
        (
            json!({
                "status": StatusCode::CONFLICT.as_u16(),
                "body": { "error": "Vibe mode is disabled by configuration for repository fixture-owner/fixture-repo." },
                "calls": [],
            }),
            false,
            Vec::<store::WatchWebQueuedPrompt>::new(),
        ),
    );
}

#[rstest]
#[tokio::test]
async fn disabling_vibe_reports_inherited_ancestor_state(action_context: ActionContext) {
    let mut parent = session("fixture-parent");
    parent.vibe_enabled = true;
    action_context.save_session(&parent);
    let mut child = session("fixture-session");
    child.launched_with_vibe = true;
    child.ancestor_session_ids = vec![parent.session_id.clone()];
    action_context.save_session(&child);

    let response = action_context
        .router_with_vibe_config(Config::default())
        .oneshot(post(
            "/sessions/fixture-session/vibe",
            json!({ "enabled": false }),
        ))
        .await
        .unwrap();
    let actual = (
        response_snapshot(response, &action_context).await,
        action_context.load_session("fixture-session").vibe_enabled,
        normalized_queued_prompts(&action_context, "fixture-session", None),
    );

    assert_eq!(
        actual,
        (
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": { "ok": true, "queuedPromptId": "<message-id>" },
                "calls": [{
                    "kind": "message",
                    "sessionId": "fixture-session",
                    "content": "watch-web からユーザーが vibe mode を off にしました。祖先セッション fixture-parent から引き継いでいるため、vibe mode は引き続き有効です。",
                }],
            }),
            false,
            json!([{
                "id": "<prompt-id>",
                "content": "watch-web からユーザーが vibe mode を off にしました。祖先セッション fixture-parent から引き継いでいるため、vibe mode は引き続き有効です。",
                "sentAt": "<sent-at>",
            }]),
        ),
    );
}
