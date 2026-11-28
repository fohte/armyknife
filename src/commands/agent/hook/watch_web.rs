use std::path::Path;

use crate::commands::agent::store;
use crate::commands::agent::types::Session;

pub(super) fn claim_launch_prompt(sessions_dir: &Path, session: &Session) {
    let Some(launch_id) = session.launch_id.as_deref() else {
        return;
    };
    if let Err(error) =
        store::claim_watch_web_launch_prompt_from(sessions_dir, launch_id, &session.session_id)
    {
        tracing::warn!(
            event = "agent.hook.watch_web_launch_prompt_claim_failed",
            session = %session.session_id,
            launch_id,
            error = %error,
        );
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use rstest::rstest;

    use super::super::side_effects::SideEffects;
    use crate::commands::agent::store;
    use crate::commands::agent::types::{HookEvent, HookInput};
    use crate::shared::env_var::EnvVars;

    #[rstest]
    #[case::matching_launch_id_on_startup(Some("fixture-launch"), Some("startup"), true)]
    #[case::missing_launch_id(None, Some("startup"), false)]
    #[case::clear_does_not_claim(Some("fixture-launch"), Some("clear"), false)]
    fn session_start_claims_the_initial_prompt_only_for_a_startup_launch_id(
        #[case] launch_id: Option<&str>,
        #[case] source: Option<&str>,
        #[case] expected_claimed: bool,
    ) {
        let sessions_dir = tempfile::TempDir::new().expect("sessions directory should be created");
        let launch_session_id = store::watch_web_launch_session_id("fixture-launch");
        let session_id = "test-123";
        let sent_at = Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).single().unwrap();
        store::insert_watch_web_queued_prompt_from(
            sessions_dir.path(),
            &launch_session_id,
            &launch_session_id,
            "Inspect this image.",
            &[store::WatchWebQueuedAttachment::Image {
                source: "/api/images/fixture.png".to_string(),
            }],
            None,
            sent_at,
        )
        .expect("queued prompt should be inserted");
        let input: HookInput = serde_json::from_value(serde_json::json!({
            "session_id": session_id,
            "cwd": "/fixture/project",
            "source": source,
        }))
        .expect("session-start input should be valid");

        super::super::process_hook_event_impl_with_env(
            HookEvent::SessionStart,
            input,
            sessions_dir.path(),
            &SideEffects::none(),
            None,
            EnvVars {
                session_launch_id: launch_id.map(str::to_string),
                ..EnvVars::default()
            },
        )
        .expect("SessionStart should save the session and claim the prompt");

        let expected_prompt = store::WatchWebQueuedPrompt {
            id: 1,
            message_id: Some(launch_session_id.clone()),
            content: "Inspect this image.".to_string(),
            action_event_kind: None,
            attachments: vec![store::WatchWebQueuedAttachment::Image {
                source: "/api/images/fixture.png".to_string(),
            }],
            resolved: false,
            sent_at,
        };
        assert_eq!(
            (
                store::watch_web_queued_prompts_from(sessions_dir.path(), &launch_session_id)
                    .expect("unclaimed queued prompts should load"),
                store::watch_web_queued_prompts_from(sessions_dir.path(), session_id)
                    .expect("claimed queued prompts should load"),
            ),
            (
                (!expected_claimed)
                    .then_some(expected_prompt.clone())
                    .into_iter()
                    .collect::<Vec<_>>(),
                expected_claimed
                    .then_some(expected_prompt)
                    .into_iter()
                    .collect::<Vec<_>>(),
            ),
        );
    }
}
