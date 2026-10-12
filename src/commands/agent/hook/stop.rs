use std::io::Write;

use anyhow::Result;
use serde_json::{Value, json};

use super::super::types::{Engine, HookEvent, HookInput, Session, SessionStatus};

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct StopConditions {
    pub stop_hook_active: bool,
    pub has_agent_status: bool,
    pub has_pending_work: bool,
    pub permission_pending: bool,
    pub asking_user: bool,
    pub paused: bool,
}

pub(super) fn block_reason(conditions: StopConditions, instruction: Option<&str>) -> Option<&str> {
    if conditions.stop_hook_active
        || conditions.has_agent_status
        || conditions.has_pending_work
        || conditions.permission_pending
        || conditions.asking_user
        || conditions.paused
    {
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
        StopConditions {
            stop_hook_active: input.stop_hook_active,
            has_agent_status: session.agent_status.is_some(),
            has_pending_work: session.has_pending_bg_tasks()
                || !input.background_tasks.is_empty()
                || !input.session_crons.is_empty(),
            permission_pending: was_waiting_for_input || session.has_pending_permission_requests(),
            asking_user: was_asking_user,
            paused: session.status == SessionStatus::Paused,
        },
        instruction,
    )
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
