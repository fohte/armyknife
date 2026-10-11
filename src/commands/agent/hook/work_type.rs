use super::super::types::{Engine, HookEvent, HookInput, Session};
use crate::shared::config::AgentConfig;

pub(super) fn may_contain_work_type(event: HookEvent, input: &HookInput) -> bool {
    work_type_candidate(event, input).is_some()
}

pub(super) fn update_session_work_type(
    session: &mut Session,
    event: HookEvent,
    input: &HookInput,
    agent_config: &AgentConfig,
) {
    if session.work_type_pinned {
        return;
    }

    let skill_name = match work_type_candidate(event, input) {
        Some(WorkTypeCandidate::ClaudePrompt(prompt)) => claude_prompt_skill(prompt)
            .filter(|skill_name| agent_config.work_type(skill_name).is_some())
            .map(str::to_owned),
        Some(WorkTypeCandidate::ClaudeSkill(skill_name)) => agent_config
            .work_type(skill_name)
            .map(|_| skill_name.to_owned()),
        Some(WorkTypeCandidate::CodexPrompt(prompt)) => codex_prompt_skill(prompt, agent_config),
        Some(WorkTypeCandidate::CodexCommand(command)) => {
            codex_command_skill(command, agent_config)
        }
        None => None,
    };

    if let Some(skill_name) = skill_name {
        session.work_type = Some(skill_name);
    }
}

enum WorkTypeCandidate<'a> {
    ClaudePrompt(&'a str),
    ClaudeSkill(&'a str),
    CodexPrompt(&'a str),
    CodexCommand(&'a str),
}

fn work_type_candidate(event: HookEvent, input: &HookInput) -> Option<WorkTypeCandidate<'_>> {
    // Work type describes the main thread's workflow, while subagent hooks share its session.
    if input.agent_id.is_some() {
        return None;
    }

    match (input.engine, event) {
        (Engine::Claude, HookEvent::UserPromptSubmit) => input
            .prompt
            .as_deref()
            .filter(|prompt| prompt.trim_start().starts_with('/'))
            .map(WorkTypeCandidate::ClaudePrompt),
        (Engine::Claude, HookEvent::PostToolUse) if input.tool_name.as_deref() == Some("Skill") => {
            input
                .tool_input
                .as_ref()?
                .skill
                .as_deref()
                .map(WorkTypeCandidate::ClaudeSkill)
        }
        (Engine::Codex, HookEvent::UserPromptSubmit) => input
            .prompt
            .as_deref()
            .filter(|prompt| prompt.contains('$'))
            .map(WorkTypeCandidate::CodexPrompt),
        (Engine::Codex, HookEvent::PostToolUse) if input.tool_name.as_deref() == Some("Bash") => {
            input
                .tool_input
                .as_ref()?
                .command
                .as_deref()
                .filter(|command| command.contains("SKILL.md"))
                .map(WorkTypeCandidate::CodexCommand)
        }
        _ => None,
    }
}

fn claude_prompt_skill(prompt: &str) -> Option<&str> {
    prompt.split_whitespace().next()?.strip_prefix('/')
}

fn codex_prompt_skill(prompt: &str, agent_config: &AgentConfig) -> Option<String> {
    prompt.match_indices('$').find_map(|(offset, _)| {
        let remaining = &prompt[offset + 1..];
        let skill_name: String = remaining
            .chars()
            .take_while(|character| is_skill_name_character(*character))
            .collect();
        (!skill_name.is_empty() && agent_config.work_type(&skill_name).is_some())
            .then_some(skill_name)
    })
}

fn codex_command_skill(command: &str, agent_config: &AgentConfig) -> Option<String> {
    let words = shlex::split(command)
        .unwrap_or_else(|| command.split_whitespace().map(str::to_owned).collect());

    words.iter().find_map(|word| {
        let skill_name = skill_name_from_path(word)?;
        agent_config
            .work_type(skill_name)
            .map(|_| skill_name.to_owned())
    })
}

fn skill_name_from_path(word: &str) -> Option<&str> {
    let path = word.trim_matches(|character| {
        matches!(
            character,
            '\'' | '"' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';'
        )
    });
    let mut components = path.rsplit('/');
    if components.next()? != "SKILL.md" {
        return None;
    }
    components
        .next()
        .filter(|skill_name| !skill_name.is_empty())
}

fn is_skill_name_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use chrono::Utc;
    use rstest::{fixture, rstest};
    use serde_json::{Value, json};

    use super::*;
    use crate::commands::agent::types::SessionStatus;
    use crate::shared::config::{AgentWorkTypeColor, AgentWorkTypeConfig, AgentWorkTypeNamedColor};

    #[fixture]
    fn agent_config() -> AgentConfig {
        let mut config = AgentConfig::default();
        for skill_name in ["flow-one", "flow-two"] {
            config.work_types.insert(
                skill_name.to_owned(),
                AgentWorkTypeConfig {
                    icon: "*".to_owned(),
                    color: AgentWorkTypeColor::Named(AgentWorkTypeNamedColor::Cyan),
                },
            );
        }
        config
    }

    type HookInputFactory = fn(Value) -> HookInput;

    #[fixture]
    fn hook_input_factory() -> HookInputFactory {
        create_hook_input
    }

    fn create_hook_input(extra: Value) -> HookInput {
        let mut payload = json!({
            "session_id": "session-test",
            "cwd": "/tmp/test",
        });
        payload
            .as_object_mut()
            .expect("object")
            .extend(extra.as_object().expect("object").clone());
        serde_json::from_value(payload).expect("valid hook input")
    }

    type SessionFactory = fn(Option<&str>) -> Session;

    #[fixture]
    fn session_factory() -> SessionFactory {
        create_session
    }

    fn create_session(work_type: Option<&str>) -> Session {
        let now = Utc::now();
        Session {
            session_id: "session-test".to_owned(),
            crit_urls: Vec::new(),
            cwd: "/tmp/test".into(),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            created_at: now,
            updated_at: now,
            last_message: None,
            current_tool: None,
            label: None,
            work_type: work_type.map(str::to_owned),
            work_type_pinned: false,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: BTreeSet::new(),
            pending_human_review_ids: BTreeSet::new(),
            pending_agent_task_ids: BTreeSet::new(),
            pending_permission_agent_ids: BTreeSet::new(),
            pending_permission_request_ids: BTreeMap::new(),
            read_at: None,
            sweep_signaled: false,
            agent_status: None,
            engine: Engine::Claude,
        }
    }

    #[rstest]
    #[case::claude_slash_prompt(
        HookEvent::UserPromptSubmit,
        json!({"engine": "claude", "prompt": "  /flow-one continue"}),
        Some("flow-two"),
        Some("flow-one"),
    )]
    #[case::claude_skill_tool(
        HookEvent::PostToolUse,
        json!({
            "engine": "claude",
            "tool_name": "Skill",
            "tool_input": {"skill": "flow-two", "args": "continue"},
        }),
        Some("flow-one"),
        Some("flow-two"),
    )]
    #[case::codex_prompt_mention(
        HookEvent::UserPromptSubmit,
        json!({"engine": "codex", "prompt": "Please use $flow-two for this task."}),
        Some("flow-one"),
        Some("flow-two"),
    )]
    #[case::codex_shell_skill_read(
        HookEvent::PostToolUse,
        json!({
            "engine": "codex",
            "tool_name": "Bash",
            "tool_input": {
                "command": "cat \"$HOME/.codex/skills/flow-one/SKILL.md\"",
            },
        }),
        Some("flow-two"),
        Some("flow-one"),
    )]
    #[case::claude_subagent_skill_tool(
        HookEvent::PostToolUse,
        json!({
            "engine": "claude",
            "agent_id": "agent-helper",
            "tool_name": "Skill",
            "tool_input": {"skill": "flow-two", "args": "continue"},
        }),
        Some("flow-one"),
        Some("flow-one"),
    )]
    #[case::codex_subagent_shell_skill_read(
        HookEvent::PostToolUse,
        json!({
            "engine": "codex",
            "agent_id": "agent-helper",
            "tool_name": "Bash",
            "tool_input": {"command": "cat /skills/flow-two/SKILL.md"},
        }),
        Some("flow-one"),
        Some("flow-one"),
    )]
    fn updates_work_type_for_configured_skills(
        agent_config: AgentConfig,
        hook_input_factory: HookInputFactory,
        session_factory: SessionFactory,
        #[case] event: HookEvent,
        #[case] extra: Value,
        #[case] initial: Option<&str>,
        #[case] expected: Option<&str>,
    ) {
        let mut session = session_factory(initial);
        let input = hook_input_factory(extra);

        update_session_work_type(&mut session, event, &input, &agent_config);

        assert_eq!(session.work_type, expected.map(str::to_owned));
    }

    #[rstest]
    #[case::claude_skill_tool(
        HookEvent::PostToolUse,
        json!({
            "engine": "claude",
            "tool_name": "Skill",
            "tool_input": {"skill": "flow-two", "args": "continue"},
        }),
    )]
    #[case::codex_shell_skill_read(
        HookEvent::PostToolUse,
        json!({
            "engine": "codex",
            "tool_name": "Bash",
            "tool_input": {
                "command": "cat \"$HOME/.codex/skills/flow-two/SKILL.md\"",
            },
        }),
    )]
    #[case::claude_slash_prompt(
        HookEvent::UserPromptSubmit,
        json!({"engine": "claude", "prompt": "/flow-two continue"}),
    )]
    #[case::codex_prompt_mention(
        HookEvent::UserPromptSubmit,
        json!({"engine": "codex", "prompt": "Please use $flow-two for this task."}),
    )]
    fn preserves_pinned_work_type(
        agent_config: AgentConfig,
        hook_input_factory: HookInputFactory,
        session_factory: SessionFactory,
        #[case] event: HookEvent,
        #[case] extra: Value,
    ) {
        let mut session = session_factory(Some("flow-one"));
        session.work_type_pinned = true;
        let input = hook_input_factory(extra);
        let normalize = |session: &Session| {
            let mut value = serde_json::to_value(session).expect("session should serialize");
            value["created_at"] = json!("<timestamp>");
            value["updated_at"] = json!("<timestamp>");
            value
        };
        let expected = normalize(&session);

        update_session_work_type(&mut session, event, &input, &agent_config);

        assert_eq!(normalize(&session), expected);
    }

    #[rstest]
    #[case::claude_slash_prompt(
        HookEvent::UserPromptSubmit,
        json!({"engine": "claude", "prompt": "/flow-one continue"}),
        true,
    )]
    #[case::claude_skill_tool(
        HookEvent::PostToolUse,
        json!({"engine": "claude", "tool_name": "Skill", "tool_input": {"skill": "flow-one"}}),
        true,
    )]
    #[case::codex_prompt_mention(
        HookEvent::UserPromptSubmit,
        json!({"engine": "codex", "prompt": "Please use $flow-one"}),
        true,
    )]
    #[case::codex_shell_skill_read(
        HookEvent::PostToolUse,
        json!({
            "engine": "codex",
            "tool_name": "Bash",
            "tool_input": {"command": "cat /skills/flow-one/SKILL.md"},
        }),
        true,
    )]
    #[case::claude_subagent_skill_tool(
        HookEvent::PostToolUse,
        json!({
            "engine": "claude",
            "agent_id": "agent-helper",
            "tool_name": "Skill",
            "tool_input": {"skill": "flow-one"},
        }),
        false,
    )]
    #[case::codex_subagent_shell_skill_read(
        HookEvent::PostToolUse,
        json!({
            "engine": "codex",
            "agent_id": "agent-helper",
            "tool_name": "Bash",
            "tool_input": {"command": "cat /skills/flow-one/SKILL.md"},
        }),
        false,
    )]
    #[case::claude_regular_prompt(
        HookEvent::UserPromptSubmit,
        json!({"engine": "claude", "prompt": "Continue the task"}),
        false,
    )]
    #[case::codex_regular_prompt(
        HookEvent::UserPromptSubmit,
        json!({"engine": "codex", "prompt": "Continue the task"}),
        false,
    )]
    #[case::unrelated_event(
        HookEvent::Stop,
        json!({"engine": "codex", "prompt": "$flow-one"}),
        false,
    )]
    #[case::unrelated_tool(
        HookEvent::PostToolUse,
        json!({"engine": "codex", "tool_name": "Read", "tool_input": {"command": "SKILL.md"}}),
        false,
    )]
    fn identifies_work_type_candidates(
        hook_input_factory: HookInputFactory,
        #[case] event: HookEvent,
        #[case] extra: Value,
        #[case] expected: bool,
    ) {
        let input = hook_input_factory(extra);

        assert_eq!(may_contain_work_type(event, &input), expected);
    }

    #[rstest]
    #[case::unconfigured_claude_slash(
        HookEvent::UserPromptSubmit,
        json!({"engine": "claude", "prompt": "/helper-step continue"}),
    )]
    #[case::nonleading_claude_slash(
        HookEvent::UserPromptSubmit,
        json!({"engine": "claude", "prompt": "Please use /flow-two later."}),
    )]
    #[case::unconfigured_codex_mention(
        HookEvent::UserPromptSubmit,
        json!({"engine": "codex", "prompt": "Please use $helper-step for this task."}),
    )]
    #[case::codex_mention_with_unconfigured_suffix(
        HookEvent::UserPromptSubmit,
        json!({"engine": "codex", "prompt": "Please use $flow-two-extra for this task."}),
    )]
    #[case::unconfigured_codex_shell_path(
        HookEvent::PostToolUse,
        json!({
            "engine": "codex",
            "tool_name": "Bash",
            "tool_input": {"command": "cat /tmp/skills/helper-step/SKILL.md"},
        }),
    )]
    #[case::unrelated_event(
        HookEvent::Stop,
        json!({"engine": "codex", "prompt": "$flow-two"}),
    )]
    #[case::unrelated_tool(
        HookEvent::PostToolUse,
        json!({
            "engine": "codex",
            "tool_name": "Read",
            "tool_input": {"command": "cat /tmp/skills/flow-two/SKILL.md"},
        }),
    )]
    fn preserves_work_type_without_configured_skill(
        agent_config: AgentConfig,
        hook_input_factory: HookInputFactory,
        session_factory: SessionFactory,
        #[case] event: HookEvent,
        #[case] extra: Value,
    ) {
        let mut session = session_factory(Some("flow-one"));
        let input = hook_input_factory(extra);

        update_session_work_type(&mut session, event, &input, &agent_config);

        assert_eq!(session.work_type, Some("flow-one".to_owned()));
    }
}
