use axum::http::StatusCode;
use chrono::{TimeZone, Utc};
use indoc::indoc;
use rstest::rstest;
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tower::ServiceExt;

use super::{
    ActionContext, action_context, normalized_queued_prompts, post, response_snapshot, session,
};
use crate::commands::agent::store::{WatchWebActionEventKind, WatchWebQueuedPrompt};
use crate::commands::agent::types::{Engine, SessionStatus};
use crate::commands::agent::{store, vibe};
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
        .router_with_vibe_config(allowed_config)
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
                "body": {
                    "ok": true,
                    "queuedPromptId": "<message-id>",
                    "vibeMode": { "mode": "enabled" },
                },
                "calls": [{
                    "kind": "message",
                    "sessionId": "fixture-session",
                    "content": "<armyknife-vibe-mode-on>watch-web からユーザーが vibe mode を on にしました。</armyknife-vibe-mode-on>",
                }],
            }),
            true,
            json!([{
                "id": "<prompt-id>",
                "content": "<armyknife-vibe-mode-on>watch-web からユーザーが vibe mode を on にしました。</armyknife-vibe-mode-on>",
                "sentAt": "<sent-at>",
            }]),
        ),
    );
}

#[rstest]
#[case::enabling(
    true,
    WatchWebActionEventKind::VibeModeOn,
    "Vibe mode turned on",
    "<armyknife-vibe-mode-on>watch-web からユーザーが vibe mode を on にしました。</armyknife-vibe-mode-on>"
)]
#[case::disabling(
    false,
    WatchWebActionEventKind::VibeModeOff,
    "Vibe mode turned off",
    "<armyknife-vibe-mode-off>watch-web からユーザーが vibe mode を off にしました。祖先セッションから引き継いでいる場合、vibe mode は引き続き有効です。</armyknife-vibe-mode-off>"
)]
#[tokio::test]
async fn codex_vibe_notifications_record_the_kind_and_normalized_event_text(
    action_context: ActionContext,
    #[case] enabled: bool,
    #[case] action_event_kind: WatchWebActionEventKind,
    #[case] expected_event_text: &str,
    #[case] expected_wire_content: &str,
) {
    let mut target = session("fixture-session");
    target.engine = Engine::Codex;
    if !enabled {
        let mut parent = session("fixture-parent");
        parent.vibe_enabled = true;
        action_context.save_session(&parent);
        target.launched_with_vibe = true;
        target.ancestor_session_ids = vec![parent.session_id];
    }
    action_context.save_session(&target);

    let response = action_context
        .router_with_vibe_config(allowed_config)
        .oneshot(post(
            "/sessions/fixture-session/vibe",
            json!({ "enabled": enabled }),
        ))
        .await
        .unwrap();
    let mut prompts = store::watch_web_queued_prompts_from(
        &action_context.temp_dir.path().join("sessions"),
        "fixture-session",
    )
    .unwrap();
    for prompt in &mut prompts {
        prompt.message_id = Some("<prompt-id>".to_owned());
        prompt.sent_at = Utc.with_ymd_and_hms(2026, 3, 14, 9, 26, 53).unwrap();
    }
    let expected_vibe_mode = if enabled {
        json!({ "mode": "enabled" })
    } else {
        json!({ "mode": "inherited", "ancestorSessionId": "fixture-parent" })
    };

    assert_eq!(
        (
            response_snapshot(response, &action_context).await,
            action_context.load_session("fixture-session").vibe_enabled,
            prompts,
        ),
        (
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": {
                    "ok": true,
                    "queuedPromptId": "<message-id>",
                    "vibeMode": expected_vibe_mode,
                },
                "calls": [{
                    "kind": "message",
                    "sessionId": "fixture-session",
                    "content": expected_wire_content,
                }],
            }),
            enabled,
            vec![WatchWebQueuedPrompt {
                id: 1,
                message_id: Some("<prompt-id>".to_owned()),
                content: expected_event_text.to_owned(),
                action_event_kind: Some(action_event_kind),
                attachments: Vec::new(),
                resolved: false,
                sent_at: Utc.with_ymd_and_hms(2026, 3, 14, 9, 26, 53).unwrap(),
            }],
        ),
    );
}

#[rstest]
#[case::enabling(
    false,
    true,
    false,
    "<armyknife-vibe-mode-on>watch-web からユーザーが vibe mode を on にしました。</armyknife-vibe-mode-on>"
)]
#[case::disabling(
    true,
    false,
    true,
    "<armyknife-vibe-mode-off>watch-web からユーザーが vibe mode を off にしました。祖先セッションから引き継いでいる場合、vibe mode は引き続き有効です。</armyknife-vibe-mode-off>"
)]
#[tokio::test]
async fn failed_prompt_delivery_restores_the_previous_vibe_setting(
    action_context: ActionContext,
    #[case] previous_enabled: bool,
    #[case] requested_enabled: bool,
    #[case] expected_enabled: bool,
    #[case] expected_content: &str,
) {
    let mut target_session = session("fixture-session");
    target_session.vibe_enabled = previous_enabled;
    action_context.save_session(&target_session);
    action_context.set_message_error("message transport failed");

    let response = action_context
        .router_with_vibe_config(allowed_config)
        .oneshot(post(
            "/sessions/fixture-session/vibe",
            json!({ "enabled": requested_enabled }),
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
                "status": StatusCode::BAD_GATEWAY.as_u16(),
                "body": { "error": "message transport failed" },
                "calls": [{
                    "kind": "message",
                    "sessionId": "fixture-session",
                    "content": expected_content,
                }],
            }),
            expected_enabled,
            Vec::<store::WatchWebQueuedPrompt>::new(),
        ),
    );
}

#[rstest]
#[tokio::test]
async fn failed_delivery_does_not_restore_over_a_concurrent_vibe_setting(
    action_context: ActionContext,
) {
    action_context.save_session(&session("fixture-session"));
    action_context.set_message_error("message transport failed");
    let sessions_dir = action_context.temp_dir.path().join("sessions");
    action_context.set_after_message(move || {
        vibe::set_in(&sessions_dir, "fixture-session", true).unwrap();
    });

    let response = action_context
        .router_with_vibe_config(allowed_config)
        .oneshot(post(
            "/sessions/fixture-session/vibe",
            json!({ "enabled": true }),
        ))
        .await
        .unwrap();
    let actual = (
        response_snapshot(response, &action_context).await,
        action_context.load_session("fixture-session").vibe_enabled,
    );

    assert_eq!(
        actual,
        (
            json!({
                "status": StatusCode::BAD_GATEWAY.as_u16(),
                "body": { "error": "message transport failed" },
                "calls": [{
                    "kind": "message",
                    "sessionId": "fixture-session",
                    "content": "<armyknife-vibe-mode-on>watch-web からユーザーが vibe mode を on にしました。</armyknife-vibe-mode-on>",
                }],
            }),
            true,
        ),
    );
}

#[rstest]
#[tokio::test]
async fn failed_delivery_restores_vibe_after_an_unrelated_session_touch(
    action_context: ActionContext,
) {
    action_context.save_session(&session("fixture-session"));
    action_context.set_message_error("message transport failed");
    let sessions_dir = action_context.temp_dir.path().join("sessions");
    action_context.set_after_message(move || {
        store::touch_session_in(&sessions_dir, "fixture-session").unwrap();
    });

    let response = action_context
        .router_with_vibe_config(allowed_config)
        .oneshot(post(
            "/sessions/fixture-session/vibe",
            json!({ "enabled": true }),
        ))
        .await
        .unwrap();
    let actual = (
        response_snapshot(response, &action_context).await,
        action_context.load_session("fixture-session").vibe_enabled,
    );

    assert_eq!(
        actual,
        (
            json!({
                "status": StatusCode::BAD_GATEWAY.as_u16(),
                "body": { "error": "message transport failed" },
                "calls": [{
                    "kind": "message",
                    "sessionId": "fixture-session",
                    "content": "<armyknife-vibe-mode-on>watch-web からユーザーが vibe mode を on にしました。</armyknife-vibe-mode-on>",
                }],
            }),
            false,
        ),
    );
}

#[rstest]
#[tokio::test]
async fn enabling_vibe_is_rejected_when_repo_policy_disallows_it(action_context: ActionContext) {
    action_context.save_session(&session("fixture-session"));

    let response = action_context
        .router_with_vibe_config(Config::default)
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
async fn enabling_vibe_uses_the_current_repo_policy(action_context: ActionContext) {
    action_context.save_session(&session("fixture-session"));
    let allow_policy = Arc::new(AtomicBool::new(false));
    let allow_policy_for_config = Arc::clone(&allow_policy);
    let router = action_context.router_with_vibe_config(move || {
        if allow_policy_for_config.load(Ordering::Relaxed) {
            allowed_config()
        } else {
            Config::default()
        }
    });

    let rejected = router
        .clone()
        .oneshot(post(
            "/sessions/fixture-session/vibe",
            json!({ "enabled": true }),
        ))
        .await
        .unwrap();
    let rejected = response_snapshot(rejected, &action_context).await;
    allow_policy.store(true, Ordering::Relaxed);
    let accepted = router
        .oneshot(post(
            "/sessions/fixture-session/vibe",
            json!({ "enabled": true }),
        ))
        .await
        .unwrap();
    let actual = (
        rejected,
        response_snapshot(accepted, &action_context).await,
        action_context.load_session("fixture-session").vibe_enabled,
    );

    assert_eq!(
        actual,
        (
            json!({
                "status": StatusCode::CONFLICT.as_u16(),
                "body": { "error": "Vibe mode is disabled by configuration for repository fixture-owner/fixture-repo." },
                "calls": [],
            }),
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": {
                    "ok": true,
                    "queuedPromptId": "<message-id>",
                    "vibeMode": { "mode": "enabled" },
                },
                "calls": [{
                    "kind": "message",
                    "sessionId": "fixture-session",
                    "content": "<armyknife-vibe-mode-on>watch-web からユーザーが vibe mode を on にしました。</armyknife-vibe-mode-on>",
                }],
            }),
            true,
        ),
    );
}

#[rstest]
#[case::ended(SessionStatus::Ended, false, "session has ended")]
#[case::stopping(SessionStatus::Running, true, "session is being stopped")]
#[tokio::test]
async fn enabling_vibe_is_rejected_for_inactive_sessions(
    action_context: ActionContext,
    #[case] status: SessionStatus,
    #[case] sweep_signaled: bool,
    #[case] expected_error: &str,
) {
    let mut session = session("fixture-session");
    session.status = status;
    session.sweep_signaled = sweep_signaled;
    action_context.save_session(&session);

    let response = action_context
        .router_with_vibe_config(Config::default)
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
                "body": { "error": expected_error },
                "calls": [],
            }),
            false,
            Vec::<store::WatchWebQueuedPrompt>::new(),
        ),
    );
}

#[rstest]
#[tokio::test]
async fn enabling_vibe_is_rejected_when_repository_cannot_be_identified(
    action_context: ActionContext,
) {
    action_context.save_session(&session("fixture-session"));

    let response = action_context
        .router_with_vibe_context(Config::default, |_| None)
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
                "body": { "error": "vibe mode is disabled because the session repository could not be identified" },
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
        .router_with_vibe_config(allowed_config)
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
                "body": {
                    "ok": true,
                    "queuedPromptId": "<message-id>",
                    "vibeMode": {
                        "mode": "inherited",
                        "ancestorSessionId": "fixture-parent",
                    },
                },
                "calls": [{
                    "kind": "message",
                    "sessionId": "fixture-session",
                    "content": "<armyknife-vibe-mode-off>watch-web からユーザーが vibe mode を off にしました。祖先セッションから引き継いでいる場合、vibe mode は引き続き有効です。</armyknife-vibe-mode-off>",
                }],
            }),
            false,
            json!([{
                "id": "<prompt-id>",
                "content": "<armyknife-vibe-mode-off>watch-web からユーザーが vibe mode を off にしました。祖先セッションから引き継いでいる場合、vibe mode は引き続き有効です。</armyknife-vibe-mode-off>",
                "sentAt": "<sent-at>",
            }]),
        ),
    );
}
