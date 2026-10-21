use std::io::Write;

use anyhow::Result;
use serde_json::{Value, json};

use super::super::types::{Engine, HookEvent, HookInput, Session, SessionStatus};
use crate::shared::config::AgentConfig;

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct StopConditions {
    pub stop_hook_active: bool,
    pub has_agent_status: bool,
    pub has_pending_work: bool,
    pub permission_pending: bool,
    pub asking_user: bool,
    pub paused: bool,
}

pub(super) fn configured_instruction(agent_config: &AgentConfig) -> Option<&str> {
    if agent_config.statuses.is_empty() {
        None
    } else {
        agent_config.stop_hook_instruction.as_deref()
    }
}

pub(super) fn configured_status_inference_prompt(agent_config: &AgentConfig) -> Option<&str> {
    if agent_config.statuses.is_empty() {
        return None;
    }

    agent_config
        .status_inference_prompt
        .as_deref()
        .filter(|prompt| !prompt.trim().is_empty())
}

pub(super) fn block_reason(conditions: StopConditions, instruction: Option<&str>) -> Option<&str> {
    if has_known_stop_reason(conditions) {
        None
    } else {
        instruction
    }
}

pub(super) fn block_reason_for_hook<'a>(
    input: &HookInput,
    session: &Session,
    was_waiting_for_input: bool,
    was_asking_user: bool,
    instruction: Option<&'a str>,
) -> Option<&'a str> {
    block_reason(
        stop_conditions_for_hook(input, session, was_waiting_for_input, was_asking_user),
        instruction,
    )
}

pub(super) fn should_infer_status_for_hook(
    input: &HookInput,
    session: &Session,
    was_waiting_for_input: bool,
    was_asking_user: bool,
    inference_prompt: Option<&str>,
) -> bool {
    should_infer_status(
        stop_conditions_for_hook(input, session, was_waiting_for_input, was_asking_user),
        inference_prompt,
    )
}

fn should_infer_status(conditions: StopConditions, inference_prompt: Option<&str>) -> bool {
    inference_prompt.is_some() && !has_known_stop_reason(conditions)
}

fn stop_conditions_for_hook(
    input: &HookInput,
    session: &Session,
    was_waiting_for_input: bool,
    was_asking_user: bool,
) -> StopConditions {
    StopConditions {
        stop_hook_active: input.stop_hook_active,
        has_agent_status: session.agent_status.is_some(),
        has_pending_work: session.has_pending_bg_tasks()
            || !input.background_tasks.is_empty()
            || !input.session_crons.is_empty(),
        permission_pending: was_waiting_for_input || session.has_pending_permission_requests(),
        asking_user: was_asking_user,
        paused: session.status == SessionStatus::Paused,
    }
}

fn has_known_stop_reason(conditions: StopConditions) -> bool {
    conditions.stop_hook_active
        || conditions.has_agent_status
        || conditions.has_pending_work
        || conditions.permission_pending
        || conditions.asking_user
        || conditions.paused
}

/// A status-setting continuation may emit intermediate hooks before Stop, so
/// keep the blocked turn's final message until that Stop arrives.
pub(super) fn preserve_last_message_after_block(
    session: &mut Session,
    event: HookEvent,
    blocked_by_armyknife: bool,
) -> bool {
    let preserve = session.stop_blocked_by_armyknife;
    if event == HookEvent::Stop {
        session.stop_blocked_by_armyknife = blocked_by_armyknife;
    }
    preserve
}

pub(super) fn write_response(
    engine: Engine,
    event: HookEvent,
    block_reason: Option<&str>,
    mut writer: impl Write,
) -> Result<()> {
    if event != HookEvent::Stop {
        return Ok(());
    }

    let Some(response) = response_value(engine, block_reason) else {
        return Ok(());
    };

    serde_json::to_writer(&mut writer, &response)?;
    writeln!(writer)?;
    writer.flush()?;
    Ok(())
}

fn response_value(engine: Engine, block_reason: Option<&str>) -> Option<Value> {
    match block_reason {
        Some(reason) => Some(json!({
            "decision": "block",
            "reason": reason,
        })),
        None if engine == Engine::Codex => Some(json!({})),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    const TEST_INSTRUCTION: &str = "Please summarize the result before stopping.";

    #[rstest]
    #[case::configured(true, Some("infer the status"), Some("infer the status"))]
    #[case::no_statuses(false, Some("infer the status"), None)]
    #[case::empty_prompt(true, Some("  "), None)]
    #[case::missing_prompt(true, None, None)]
    fn status_inference_prompt_requires_configured_statuses(
        #[case] has_statuses: bool,
        #[case] prompt: Option<&str>,
        #[case] expected: Option<&str>,
    ) {
        let mut agent_config = crate::shared::config::test_support::agent_config();
        if !has_statuses {
            agent_config.statuses.clear();
        }
        agent_config.status_inference_prompt = prompt.map(str::to_string);

        assert_eq!(configured_status_inference_prompt(&agent_config), expected);
    }

    #[derive(Debug, Clone, Copy)]
    enum KnownStopReason {
        StopHookAlreadyContinued,
        StatusSet,
        BackgroundWorkPending,
        PermissionPending,
        AskingUser,
        Paused,
    }

    #[rstest]
    #[case::stop_hook_already_continued(KnownStopReason::StopHookAlreadyContinued)]
    #[case::status_set(KnownStopReason::StatusSet)]
    #[case::background_work_pending(KnownStopReason::BackgroundWorkPending)]
    #[case::permission_pending(KnownStopReason::PermissionPending)]
    #[case::asking_user(KnownStopReason::AskingUser)]
    #[case::paused(KnownStopReason::Paused)]
    fn known_stop_reasons_do_not_block(#[case] reason: KnownStopReason) {
        let mut conditions = StopConditions::default();
        match reason {
            KnownStopReason::StopHookAlreadyContinued => conditions.stop_hook_active = true,
            KnownStopReason::StatusSet => conditions.has_agent_status = true,
            KnownStopReason::BackgroundWorkPending => conditions.has_pending_work = true,
            KnownStopReason::PermissionPending => conditions.permission_pending = true,
            KnownStopReason::AskingUser => conditions.asking_user = true,
            KnownStopReason::Paused => conditions.paused = true,
        }

        assert_eq!(block_reason(conditions, Some(TEST_INSTRUCTION)), None);
    }

    #[rstest]
    #[case::configured(Some(TEST_INSTRUCTION), Some(TEST_INSTRUCTION))]
    #[case::missing(None, None)]
    fn unknown_stop_returns_only_a_configured_instruction(
        #[case] instruction: Option<&str>,
        #[case] expected: Option<&str>,
    ) {
        assert_eq!(
            block_reason(StopConditions::default(), instruction),
            expected
        );
    }

    #[rstest]
    #[case::stop_hook_already_continued(KnownStopReason::StopHookAlreadyContinued)]
    #[case::status_set(KnownStopReason::StatusSet)]
    #[case::background_work_pending(KnownStopReason::BackgroundWorkPending)]
    #[case::permission_pending(KnownStopReason::PermissionPending)]
    #[case::asking_user(KnownStopReason::AskingUser)]
    #[case::paused(KnownStopReason::Paused)]
    fn known_stop_reasons_do_not_start_inference(#[case] reason: KnownStopReason) {
        let mut conditions = StopConditions::default();
        match reason {
            KnownStopReason::StopHookAlreadyContinued => conditions.stop_hook_active = true,
            KnownStopReason::StatusSet => conditions.has_agent_status = true,
            KnownStopReason::BackgroundWorkPending => conditions.has_pending_work = true,
            KnownStopReason::PermissionPending => conditions.permission_pending = true,
            KnownStopReason::AskingUser => conditions.asking_user = true,
            KnownStopReason::Paused => conditions.paused = true,
        }

        assert!(!should_infer_status(conditions, Some("prompt")));
    }

    #[rstest]
    #[case::configured(Some("prompt"), true)]
    #[case::missing(None, false)]
    fn status_inference_requires_a_prompt(#[case] prompt: Option<&str>, #[case] expected: bool) {
        assert_eq!(
            should_infer_status(StopConditions::default(), prompt),
            expected
        );
    }

    #[rstest]
    #[case::claude(Engine::Claude)]
    #[case::codex(Engine::Codex)]
    fn stop_block_writes_the_complete_response(#[case] engine: Engine) {
        let mut output = Vec::new();
        write_response(engine, HookEvent::Stop, Some(TEST_INSTRUCTION), &mut output)
            .expect("response should write");

        let mut expected = serde_json::to_vec(&json!({
            "decision": "block",
            "reason": TEST_INSTRUCTION,
        }))
        .expect("expected response should serialize");
        expected.push(b'\n');
        assert_eq!(output, expected);
    }

    #[rstest]
    #[case::claude_stop(Engine::Claude, HookEvent::Stop, b"")]
    #[case::codex_stop(Engine::Codex, HookEvent::Stop, b"{}\n")]
    #[case::claude_stop_failure(Engine::Claude, HookEvent::StopFailure, b"")]
    #[case::codex_stop_failure(Engine::Codex, HookEvent::StopFailure, b"")]
    #[case::codex_other_event(Engine::Codex, HookEvent::PostToolUse, b"")]
    fn allowed_or_other_events_write_expected_output(
        #[case] engine: Engine,
        #[case] event: HookEvent,
        #[case] expected: &[u8],
    ) {
        let mut output = Vec::new();
        write_response(engine, event, None, &mut output).expect("response should write");

        assert_eq!(output, expected);
    }
}
