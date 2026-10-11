use std::io::{self, Write};

use anyhow::Result;
use serde_json::{Value, json};

use super::super::types::{Engine, HookInput, Session, SessionStatus};

const STATUS_INSTRUCTION: &str = indoc::indoc! {"
    Set your status with `a agent status set <kind> <note>` before stopping.
    Write a one-line note describing what you need or what is complete.
    The first five kinds mean it is the user's turn; `done` and `wait` do not:
    - `decide`: Ask the user to choose between options or set a direction.
    - `read`: Answer the user's question and finish.
    - `do`: Wait for the user to perform a manual action, such as verification or registration.
    - `idle`: Wait for the user's next instruction.
    - `close`: Only closing remains; it is not urgent.
    - `done`: Report completion to the parent agent.
    - `wait`: Wait for another session, CI, or other external work to finish.
"};

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct StopConditions {
    pub stop_hook_active: bool,
    pub has_agent_status: bool,
    pub has_pending_work: bool,
    pub permission_pending: bool,
    pub asking_user: bool,
    pub paused: bool,
}

pub(super) fn block_reason(conditions: StopConditions) -> Option<&'static str> {
    if conditions.stop_hook_active
        || conditions.has_agent_status
        || conditions.has_pending_work
        || conditions.permission_pending
        || conditions.asking_user
        || conditions.paused
    {
        None
    } else {
        Some(STATUS_INSTRUCTION)
    }
}

pub(super) fn block_reason_for_hook(
    input: &HookInput,
    session: &Session,
    was_waiting_for_input: bool,
    was_asking_user: bool,
) -> Option<&'static str> {
    block_reason(StopConditions {
        stop_hook_active: input.stop_hook_active,
        has_agent_status: session.agent_status.is_some(),
        has_pending_work: session.has_pending_bg_tasks()
            || !input.background_tasks.is_empty()
            || !input.session_crons.is_empty(),
        permission_pending: was_waiting_for_input || session.has_pending_permission_requests(),
        asking_user: was_asking_user,
        paused: session.status == SessionStatus::Paused,
    })
}

pub(super) fn write_response(
    engine: Engine,
    event_name: &str,
    block_reason: Option<&str>,
) -> Result<()> {
    if event_name != "stop" {
        return Ok(());
    }

    let Some(response) = response_value(engine, block_reason) else {
        return Ok(());
    };

    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, &response)?;
    writeln!(stdout)?;
    stdout.flush()?;
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
    use indoc::indoc;
    use rstest::rstest;

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

        assert_eq!(block_reason(conditions), None);
    }

    #[test]
    fn unknown_stop_requires_status_with_all_kind_meanings() {
        assert_eq!(
            block_reason(StopConditions::default()),
            Some(indoc! {"
                Set your status with `a agent status set <kind> <note>` before stopping.
                Write a one-line note describing what you need or what is complete.
                The first five kinds mean it is the user's turn; `done` and `wait` do not:
                - `decide`: Ask the user to choose between options or set a direction.
                - `read`: Answer the user's question and finish.
                - `do`: Wait for the user to perform a manual action, such as verification or registration.
                - `idle`: Wait for the user's next instruction.
                - `close`: Only closing remains; it is not urgent.
                - `done`: Report completion to the parent agent.
                - `wait`: Wait for another session, CI, or other external work to finish.
            "})
        );
    }

    #[rstest]
    #[case::claude_block(Engine::Claude, Some(STATUS_INSTRUCTION), Some(json!({
        "decision": "block",
        "reason": STATUS_INSTRUCTION,
    })))]
    #[case::codex_block(Engine::Codex, Some(STATUS_INSTRUCTION), Some(json!({
        "decision": "block",
        "reason": STATUS_INSTRUCTION,
    })))]
    #[case::claude_continue(Engine::Claude, None, None)]
    #[case::codex_continue(Engine::Codex, None, Some(json!({})))]
    fn response_format_matches_engine(
        #[case] engine: Engine,
        #[case] block_reason: Option<&str>,
        #[case] expected: Option<Value>,
    ) {
        assert_eq!(response_value(engine, block_reason), expected);
    }
}
