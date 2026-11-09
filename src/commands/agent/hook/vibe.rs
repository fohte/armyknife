use super::super::types::{HookEvent, HookInput, Session};
use super::prompt_skill;

const ENABLE_VIBE_CODING_SKILL: &str = "enable-vibe-coding";

pub(super) fn enable_on_prompt_submit(event: HookEvent, input: &HookInput, session: &mut Session) {
    if event == HookEvent::UserPromptSubmit
        && input.agent_id.is_none()
        && input.prompt.as_deref().is_some_and(|prompt| {
            prompt_skill::leading_prompt_skill(input.engine, prompt)
                == Some(ENABLE_VIBE_CODING_SKILL)
        })
    {
        session.vibe_enabled = true;
    }
}

#[cfg(test)]
mod tests {
    use rstest::{fixture, rstest};
    use serde_json::json;

    use super::super::{SideEffects, process_hook_event_impl};
    use super::*;
    use crate::commands::agent::store;
    use crate::commands::agent::types::{Engine, SessionStatus};

    #[derive(Clone, Copy)]
    struct PromptCase {
        engine: Engine,
        event: HookEvent,
        agent_id: Option<&'static str>,
        child_launch_denied: bool,
        initially_vibe_enabled: bool,
        prompt: Option<&'static str>,
        tool_name: Option<&'static str>,
        expected: bool,
    }

    #[fixture]
    fn session_factory() -> fn(bool, bool) -> Session {
        |child_launch_denied, vibe_enabled| {
            serde_json::from_value(json!({
                "session_id": "session-test",
                "cwd": "/tmp/session-test",
                "transcript_path": null,
                "tmux_info": null,
                "status": "running",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z",
                "last_message": null,
                "child_launch": if child_launch_denied { "deny" } else { "allow" },
                "vibe_enabled": vibe_enabled,
            }))
            .expect("valid test session")
        }
    }

    fn hook_input(
        engine: Engine,
        agent_id: Option<&str>,
        prompt: Option<&str>,
        tool_name: Option<&str>,
    ) -> HookInput {
        serde_json::from_value(json!({
            "session_id": "session-test",
            "cwd": "/tmp/session-test",
            "engine": engine,
            "agent_id": agent_id,
            "prompt": prompt,
            "tool_name": tool_name,
            "tool_input": {"skill": ENABLE_VIBE_CODING_SKILL},
        }))
        .expect("valid test hook input")
    }

    #[rstest]
    #[case::claude_invocation(PromptCase {
        engine: Engine::Claude,
        event: HookEvent::UserPromptSubmit,
        agent_id: None,
        child_launch_denied: false,
        initially_vibe_enabled: false,
        prompt: Some("/enable-vibe-coding"),
        tool_name: None,
        expected: true,
    })]
    #[case::codex_invocation(PromptCase {
        engine: Engine::Codex,
        event: HookEvent::UserPromptSubmit,
        agent_id: None,
        child_launch_denied: false,
        initially_vibe_enabled: false,
        prompt: Some("  $enable-vibe-coding"),
        tool_name: None,
        expected: true,
    })]
    #[case::claude_invocation_with_arguments(PromptCase {
        engine: Engine::Claude,
        event: HookEvent::UserPromptSubmit,
        agent_id: None,
        child_launch_denied: false,
        initially_vibe_enabled: false,
        prompt: Some("/enable-vibe-coding work"),
        tool_name: None,
        expected: true,
    })]
    #[case::codex_invocation_with_arguments(PromptCase {
        engine: Engine::Codex,
        event: HookEvent::UserPromptSubmit,
        agent_id: None,
        child_launch_denied: false,
        initially_vibe_enabled: false,
        prompt: Some("$enable-vibe-coding work"),
        tool_name: None,
        expected: true,
    })]
    #[case::subagent_prompt(PromptCase {
        engine: Engine::Claude,
        event: HookEvent::UserPromptSubmit,
        agent_id: Some("agent-test"),
        child_launch_denied: false,
        initially_vibe_enabled: false,
        prompt: Some("/enable-vibe-coding"),
        tool_name: None,
        expected: false,
    })]
    #[case::denied_child_launch_does_not_disable_vibe_prompt(PromptCase {
        engine: Engine::Claude,
        event: HookEvent::UserPromptSubmit,
        agent_id: None,
        child_launch_denied: true,
        initially_vibe_enabled: false,
        prompt: Some("/enable-vibe-coding"),
        tool_name: None,
        expected: true,
    })]
    #[case::post_tool_use_skill(PromptCase {
        engine: Engine::Claude,
        event: HookEvent::PostToolUse,
        agent_id: None,
        child_launch_denied: false,
        initially_vibe_enabled: false,
        prompt: None,
        tool_name: Some("Skill"),
        expected: false,
    })]
    #[case::claude_skill_name_after_first_token(PromptCase {
        engine: Engine::Claude,
        event: HookEvent::UserPromptSubmit,
        agent_id: None,
        child_launch_denied: false,
        initially_vibe_enabled: false,
        prompt: Some("Please use /enable-vibe-coding"),
        tool_name: None,
        expected: false,
    })]
    #[case::codex_skill_name_after_first_token(PromptCase {
        engine: Engine::Codex,
        event: HookEvent::UserPromptSubmit,
        agent_id: None,
        child_launch_denied: false,
        initially_vibe_enabled: false,
        prompt: Some("Please use $enable-vibe-coding"),
        tool_name: None,
        expected: false,
    })]
    #[case::codex_later_skill_reference(PromptCase {
        engine: Engine::Codex,
        event: HookEvent::UserPromptSubmit,
        agent_id: None,
        child_launch_denied: false,
        initially_vibe_enabled: false,
        prompt: Some("$other-skill then $enable-vibe-coding"),
        tool_name: None,
        expected: false,
    })]
    #[case::wrong_engine_prefix(PromptCase {
        engine: Engine::Codex,
        event: HookEvent::UserPromptSubmit,
        agent_id: None,
        child_launch_denied: false,
        initially_vibe_enabled: false,
        prompt: Some("/enable-vibe-coding"),
        tool_name: None,
        expected: false,
    })]
    #[case::skill_name_prefix(PromptCase {
        engine: Engine::Claude,
        event: HookEvent::UserPromptSubmit,
        agent_id: None,
        child_launch_denied: false,
        initially_vibe_enabled: false,
        prompt: Some("/enable-vibe-coding-extra"),
        tool_name: None,
        expected: false,
    })]
    #[case::missing_prompt(PromptCase {
        engine: Engine::Claude,
        event: HookEvent::UserPromptSubmit,
        agent_id: None,
        child_launch_denied: false,
        initially_vibe_enabled: false,
        prompt: None,
        tool_name: None,
        expected: false,
    })]
    #[case::existing_vibe_is_preserved(PromptCase {
        engine: Engine::Claude,
        event: HookEvent::UserPromptSubmit,
        agent_id: None,
        child_launch_denied: false,
        initially_vibe_enabled: true,
        prompt: Some("ordinary request"),
        tool_name: None,
        expected: true,
    })]
    fn enables_vibe_only_for_a_matching_main_thread_prompt(
        session_factory: fn(bool, bool) -> Session,
        #[case] case: PromptCase,
    ) {
        let input = hook_input(case.engine, case.agent_id, case.prompt, case.tool_name);
        let mut session = session_factory(case.child_launch_denied, case.initially_vibe_enabled);

        enable_on_prompt_submit(case.event, &input, &mut session);

        assert_eq!(session.vibe_enabled, case.expected);
    }

    #[rstest]
    #[case::explicitly_enabled(true, false)]
    #[case::inherited(false, true)]
    fn sweep_pause_and_resume_keep_vibe_settings(
        session_factory: fn(bool, bool) -> Session,
        #[case] vibe_enabled: bool,
        #[case] launched_with_vibe: bool,
    ) {
        let sessions_dir = tempfile::TempDir::new().expect("temp dir creation should succeed");
        let mut session = session_factory(false, vibe_enabled);
        session.status = SessionStatus::Stopped;
        session.sweep_signaled = true;
        session.launched_with_vibe = launched_with_vibe;
        session.ancestor_session_ids = vec!["ancestor-session".to_string()];
        store::save_session_to(sessions_dir.path(), &session).expect("session should save");
        let process = |event, source: Option<&str>| {
            let input: HookInput = serde_json::from_value(json!({
                "session_id": "session-test",
                "cwd": "/tmp/session-test",
                "source": source,
            }))
            .expect("valid test hook input");
            process_hook_event_impl(event, input, sessions_dir.path(), &SideEffects::none())
                .expect("hook should succeed");
            let stored = store::load_session_from(sessions_dir.path(), "session-test")
                .expect("session should load")
                .expect("session should exist");
            (
                stored.status,
                stored.vibe_enabled,
                stored.launched_with_vibe,
                stored.ancestor_session_ids,
            )
        };

        let paused = process(HookEvent::SessionEnd, None);
        let resumed = process(HookEvent::SessionStart, Some("resume"));

        let ancestors = vec!["ancestor-session".to_string()];
        assert_eq!(
            (paused, resumed),
            (
                (
                    SessionStatus::Paused,
                    vibe_enabled,
                    launched_with_vibe,
                    ancestors.clone(),
                ),
                (
                    SessionStatus::Stopped,
                    vibe_enabled,
                    launched_with_vibe,
                    ancestors,
                ),
            ),
        );
    }
}
