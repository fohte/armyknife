use super::super::types::{HookEvent, HookInput, Session};
use super::prompt_skill;

const ENABLE_VIBE_CODING_SKILL: &str = "enable-vibe-coding";

pub(super) fn enable_on_prompt_submit(event: HookEvent, input: &HookInput, session: &mut Session) {
    if event == HookEvent::UserPromptSubmit
        && input.agent_id.is_none()
        && !session.is_delegated
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

    use super::*;
    use crate::commands::agent::types::Engine;

    #[derive(Clone, Copy)]
    struct PromptCase {
        engine: Engine,
        event: HookEvent,
        agent_id: Option<&'static str>,
        is_delegated: bool,
        initially_vibe_enabled: bool,
        prompt: Option<&'static str>,
        tool_name: Option<&'static str>,
        expected: bool,
    }

    #[fixture]
    fn session_factory() -> fn(bool, bool) -> Session {
        |is_delegated, vibe_enabled| {
            serde_json::from_value(json!({
                "session_id": "session-test",
                "cwd": "/tmp/session-test",
                "transcript_path": null,
                "tmux_info": null,
                "status": "running",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z",
                "last_message": null,
                "is_delegated": is_delegated,
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
        is_delegated: false,
        initially_vibe_enabled: false,
        prompt: Some("/enable-vibe-coding"),
        tool_name: None,
        expected: true,
    })]
    #[case::codex_invocation(PromptCase {
        engine: Engine::Codex,
        event: HookEvent::UserPromptSubmit,
        agent_id: None,
        is_delegated: false,
        initially_vibe_enabled: false,
        prompt: Some("  $enable-vibe-coding"),
        tool_name: None,
        expected: true,
    })]
    #[case::claude_invocation_with_arguments(PromptCase {
        engine: Engine::Claude,
        event: HookEvent::UserPromptSubmit,
        agent_id: None,
        is_delegated: false,
        initially_vibe_enabled: false,
        prompt: Some("/enable-vibe-coding work"),
        tool_name: None,
        expected: true,
    })]
    #[case::codex_invocation_with_arguments(PromptCase {
        engine: Engine::Codex,
        event: HookEvent::UserPromptSubmit,
        agent_id: None,
        is_delegated: false,
        initially_vibe_enabled: false,
        prompt: Some("$enable-vibe-coding work"),
        tool_name: None,
        expected: true,
    })]
    #[case::subagent_prompt(PromptCase {
        engine: Engine::Claude,
        event: HookEvent::UserPromptSubmit,
        agent_id: Some("agent-test"),
        is_delegated: false,
        initially_vibe_enabled: false,
        prompt: Some("/enable-vibe-coding"),
        tool_name: None,
        expected: false,
    })]
    #[case::delegated_prompt(PromptCase {
        engine: Engine::Claude,
        event: HookEvent::UserPromptSubmit,
        agent_id: None,
        is_delegated: true,
        initially_vibe_enabled: false,
        prompt: Some("/enable-vibe-coding"),
        tool_name: None,
        expected: false,
    })]
    #[case::post_tool_use_skill(PromptCase {
        engine: Engine::Claude,
        event: HookEvent::PostToolUse,
        agent_id: None,
        is_delegated: false,
        initially_vibe_enabled: false,
        prompt: None,
        tool_name: Some("Skill"),
        expected: false,
    })]
    #[case::claude_skill_name_after_first_token(PromptCase {
        engine: Engine::Claude,
        event: HookEvent::UserPromptSubmit,
        agent_id: None,
        is_delegated: false,
        initially_vibe_enabled: false,
        prompt: Some("Please use /enable-vibe-coding"),
        tool_name: None,
        expected: false,
    })]
    #[case::codex_skill_name_after_first_token(PromptCase {
        engine: Engine::Codex,
        event: HookEvent::UserPromptSubmit,
        agent_id: None,
        is_delegated: false,
        initially_vibe_enabled: false,
        prompt: Some("Please use $enable-vibe-coding"),
        tool_name: None,
        expected: false,
    })]
    #[case::codex_later_skill_reference(PromptCase {
        engine: Engine::Codex,
        event: HookEvent::UserPromptSubmit,
        agent_id: None,
        is_delegated: false,
        initially_vibe_enabled: false,
        prompt: Some("$other-skill then $enable-vibe-coding"),
        tool_name: None,
        expected: false,
    })]
    #[case::wrong_engine_prefix(PromptCase {
        engine: Engine::Codex,
        event: HookEvent::UserPromptSubmit,
        agent_id: None,
        is_delegated: false,
        initially_vibe_enabled: false,
        prompt: Some("/enable-vibe-coding"),
        tool_name: None,
        expected: false,
    })]
    #[case::skill_name_prefix(PromptCase {
        engine: Engine::Claude,
        event: HookEvent::UserPromptSubmit,
        agent_id: None,
        is_delegated: false,
        initially_vibe_enabled: false,
        prompt: Some("/enable-vibe-coding-extra"),
        tool_name: None,
        expected: false,
    })]
    #[case::missing_prompt(PromptCase {
        engine: Engine::Claude,
        event: HookEvent::UserPromptSubmit,
        agent_id: None,
        is_delegated: false,
        initially_vibe_enabled: false,
        prompt: None,
        tool_name: None,
        expected: false,
    })]
    #[case::existing_vibe_is_preserved(PromptCase {
        engine: Engine::Claude,
        event: HookEvent::UserPromptSubmit,
        agent_id: None,
        is_delegated: false,
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
        let mut session = session_factory(case.is_delegated, case.initially_vibe_enabled);

        enable_on_prompt_submit(case.event, &input, &mut session);

        assert_eq!(session.vibe_enabled, case.expected);
    }
}
