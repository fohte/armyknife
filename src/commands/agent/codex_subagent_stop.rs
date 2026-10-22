use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Args;
use indoc::formatdoc;
use serde::Deserialize;
use serde_json::Value;

use crate::commands::agent::codex_steer::Client;
#[cfg(not(test))]
use crate::infra::process;
use crate::shared::log::short_run_id;

mod ledger;

#[cfg(test)]
use self::ledger::EventLease;
use self::ledger::{EventKey, EventLedger, FileEventLedger};

const CHILD_TURN_POLL_INTERVAL: Duration = Duration::from_secs(1);
const MAX_CHILD_TURN_READS: usize = 600;
const MAX_QUEUE_MESSAGE_CHARS: usize = 1 << 20;
const QUEUE_MESSAGE_TRUNCATION_NOTICE: &str = "[Result truncated to fit the Codex queue input limit; inspect the referenced child turn for the full result.]";

#[derive(Args, Clone, PartialEq, Eq)]
pub struct CodexSubagentStopArgs;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct CodexSubagentStopWorkerArgs {
    #[arg(long)]
    pub session_id: String,
    #[arg(long)]
    pub child_thread_id: String,
    #[arg(long)]
    pub turn_id: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct HookInput {
    session_id: String,
    agent_id: String,
    turn_id: String,
}

impl From<&CodexSubagentStopWorkerArgs> for HookInput {
    fn from(args: &CodexSubagentStopWorkerArgs) -> Self {
        Self {
            session_id: args.session_id.clone(),
            agent_id: args.child_thread_id.clone(),
            turn_id: args.turn_id.clone(),
        }
    }
}

pub fn run(_: &CodexSubagentStopArgs) -> Result<()> {
    let input: HookInput = serde_json::from_reader(std::io::stdin().lock())
        .context("failed to read Codex SubagentStop hook input")?;
    let span = tracing::info_span!(
        "agent.codex_subagent_stop",
        run_id = %short_run_id(),
        session = %input.session_id,
    );
    let _entered = span.enter();
    spawn_worker(&input);
    Ok(())
}

pub fn run_worker(args: &CodexSubagentStopWorkerArgs) -> Result<()> {
    let input = HookInput::from(args);
    let span = tracing::info_span!(
        "agent.codex_subagent_stop",
        run_id = %short_run_id(),
        session = %input.session_id,
    );
    let _entered = span.enter();
    let result = (|| {
        let mut client = Client::connect()?;
        let ledger = FileEventLedger::new()?;
        process_event(
            &input,
            &mut client,
            &ledger,
            MAX_CHILD_TURN_READS,
            CHILD_TURN_POLL_INTERVAL,
            thread::sleep,
        )
    })();

    match result {
        Ok(Outcome::Queued) => tracing::info!(
            event = "agent.codex_subagent_stop.queued",
            session_id = %input.session_id,
            child_thread_id = %input.agent_id,
            turn_id = %input.turn_id,
        ),
        Ok(outcome) => tracing::info!(
            event = "agent.codex_subagent_stop.finished",
            session_id = %input.session_id,
            child_thread_id = %input.agent_id,
            turn_id = %input.turn_id,
            ?outcome,
        ),
        Err(error) => tracing::warn!(
            event = "agent.codex_subagent_stop.failed",
            session_id = %input.session_id,
            child_thread_id = %input.agent_id,
            turn_id = %input.turn_id,
            error = %error,
        ),
    }
    Ok(())
}

#[cfg(not(test))]
fn spawn_worker(input: &HookInput) {
    process::spawn_self_detached(
        "agent.codex_subagent_stop.spawn",
        "agent.codex_subagent_stop.spawn_failed",
        &input.session_id,
        &[
            "agent",
            "codex-subagent-stop-worker",
            "--session-id",
            &input.session_id,
            "--child-thread-id",
            &input.agent_id,
            "--turn-id",
            &input.turn_id,
        ],
    );
}

#[cfg(test)]
fn spawn_worker(_: &HookInput) {}

trait WakeParentControl {
    fn read_thread(&mut self, thread_id: &str, include_turns: bool) -> Result<Value>;
    fn queue_add(&mut self, thread_id: &str, content: &str) -> Result<()>;
}

impl WakeParentControl for Client {
    fn read_thread(&mut self, thread_id: &str, include_turns: bool) -> Result<Value> {
        self.read_subagent_thread(thread_id, include_turns)
    }

    fn queue_add(&mut self, thread_id: &str, content: &str) -> Result<()> {
        self.queue_subagent_message(thread_id, content)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ParentStatus {
    Idle,
    Active,
    NotLoaded,
    SystemError,
    Unknown(Option<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Outcome {
    Duplicate,
    Queued,
    ChildTurnPending,
    ChildTurnInterrupted,
    UnknownChildTurnStatus(String),
    MissingParent,
    ParentNotQueueable(ParentStatus),
    ParentUnavailable,
    QueueRejected,
}

fn process_event<C, L, S>(
    input: &HookInput,
    control: &mut C,
    ledger: &L,
    max_reads: usize,
    poll_interval: Duration,
    mut sleep: S,
) -> Result<Outcome>
where
    C: WakeParentControl,
    L: EventLedger,
    S: FnMut(Duration),
{
    let key = EventKey::from(input);
    let Some(mut lease) = ledger.begin(&key)? else {
        return Ok(Outcome::Duplicate);
    };

    let (child_thread, completed_turn, turn_status) =
        match wait_for_child_turn(control, input, max_reads, poll_interval, &mut sleep)? {
            ChildTurn::Completed {
                thread,
                turn,
                status,
            } => (thread, turn, status),
            ChildTurn::Pending => return Ok(Outcome::ChildTurnPending),
            ChildTurn::Interrupted => {
                lease.mark_processed()?;
                return Ok(Outcome::ChildTurnInterrupted);
            }
            ChildTurn::Unknown(status) => {
                lease.mark_processed()?;
                return Ok(Outcome::UnknownChildTurnStatus(status));
            }
        };

    let Some(parent_thread_id) = child_thread
        .pointer("/thread/parentThreadId")
        .and_then(Value::as_str)
    else {
        lease.mark_processed()?;
        return Ok(Outcome::MissingParent);
    };

    let parent_thread = match control.read_thread(parent_thread_id, false) {
        Ok(thread) => thread,
        Err(error) => {
            tracing::warn!(
                event = "agent.codex_subagent_stop.parent_read_failed",
                session_id = %input.session_id,
                child_thread_id = %input.agent_id,
                turn_id = %input.turn_id,
                parent_thread_id,
                error = %error,
            );
            lease.mark_processed()?;
            return Ok(Outcome::ParentUnavailable);
        }
    };

    let parent_status = parent_status(&parent_thread);
    if !matches!(&parent_status, ParentStatus::Idle | ParentStatus::Active) {
        lease.mark_processed()?;
        return Ok(Outcome::ParentNotQueueable(parent_status));
    }

    let message = queue_result_message(input, turn_status, &completed_turn)?;

    // Mark before the request because a lost response cannot reveal whether the
    // queue accepted it, and a duplicate hook must not add a second message.
    lease.mark_processed()?;
    match control.queue_add(parent_thread_id, &message) {
        Ok(()) => Ok(Outcome::Queued),
        Err(error) => {
            tracing::warn!(
                event = "agent.codex_subagent_stop.queue_add_failed",
                session_id = %input.session_id,
                child_thread_id = %input.agent_id,
                turn_id = %input.turn_id,
                parent_thread_id,
                error = %error,
            );
            Ok(Outcome::QueueRejected)
        }
    }
}

#[derive(Clone, Copy)]
enum TerminalTurnStatus {
    Completed,
    Failed,
}

impl TerminalTurnStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

enum ChildTurn {
    Completed {
        thread: Value,
        turn: Value,
        status: TerminalTurnStatus,
    },
    Interrupted,
    Pending,
    Unknown(String),
}

fn wait_for_child_turn<C, S>(
    control: &mut C,
    input: &HookInput,
    max_reads: usize,
    poll_interval: Duration,
    sleep: &mut S,
) -> Result<ChildTurn>
where
    C: WakeParentControl,
    S: FnMut(Duration),
{
    for read_number in 0..max_reads {
        let thread = control
            .read_thread(&input.agent_id, true)
            .with_context(|| format!("failed to read child thread {}", input.agent_id))?;
        let target_turn = thread
            .pointer("/thread/turns")
            .and_then(Value::as_array)
            .and_then(|turns| {
                turns
                    .iter()
                    .find(|turn| turn.get("id").and_then(Value::as_str) == Some(&input.turn_id))
            });
        let status = target_turn.and_then(|turn| turn.get("status").and_then(Value::as_str));
        let terminal_status = match status {
            Some("completed") => Some(TerminalTurnStatus::Completed),
            Some("failed") => Some(TerminalTurnStatus::Failed),
            _ => None,
        };

        if let (Some(turn), Some(status)) = (target_turn, terminal_status) {
            let turn = turn.clone();
            return Ok(ChildTurn::Completed {
                thread,
                turn,
                status,
            });
        }

        match status {
            Some("interrupted") => return Ok(ChildTurn::Interrupted),
            Some("inProgress") | None => {
                if read_number + 1 < max_reads {
                    sleep(poll_interval);
                }
            }
            Some(status) => return Ok(ChildTurn::Unknown(status.to_owned())),
        }
    }
    Ok(ChildTurn::Pending)
}

fn queue_result_message(
    input: &HookInput,
    status: TerminalTurnStatus,
    turn: &Value,
) -> Result<String> {
    let (result_label, result) = match status {
        TerminalTurnStatus::Completed => (
            "Final answer",
            final_answer(turn).unwrap_or("No final answer was available in the child turn."),
        ),
        TerminalTurnStatus::Failed => (
            "Failure reason",
            failure_reason(turn).unwrap_or("No failure reason was provided in the child turn."),
        ),
    };
    let mut header = formatdoc! {"
        Codex subagent turn finished before native mailbox delivery.
        Child thread ID: {}
        Turn ID: {}
        Status: {}
        {}:",
        input.agent_id,
        input.turn_id,
        status.as_str(),
        result_label,
    };
    header.push('\n');
    let header_chars = header.chars().count();
    let result_chars = result.chars().count();
    if header_chars + result_chars <= MAX_QUEUE_MESSAGE_CHARS {
        return Ok(format!("{header}{result}"));
    }

    let notice_chars = QUEUE_MESSAGE_TRUNCATION_NOTICE.chars().count() + 2;
    let result_budget = MAX_QUEUE_MESSAGE_CHARS
        .checked_sub(header_chars)
        .and_then(|remaining| remaining.checked_sub(notice_chars))
        .context("Codex subagent metadata exceeds the queue input limit")?;
    let truncated_result = result.chars().take(result_budget).collect::<String>();
    let mut message = format!("{header}{truncated_result}");
    message.push('\n');
    message.push('\n');
    message.push_str(QUEUE_MESSAGE_TRUNCATION_NOTICE);
    Ok(message)
}

fn final_answer(turn: &Value) -> Option<&str> {
    let items = turn.get("items")?.as_array()?;
    let final_answer = items.iter().rev().find(|item| {
        item.get("type").and_then(Value::as_str) == Some("agentMessage")
            && item.get("phase").and_then(Value::as_str) == Some("final_answer")
    });
    let unphased_message = items.iter().rev().find(|item| {
        item.get("type").and_then(Value::as_str) == Some("agentMessage")
            && item.get("phase").is_none_or(Value::is_null)
    });
    final_answer
        .or(unphased_message)
        .and_then(|item| item.get("text").and_then(Value::as_str))
}

fn failure_reason(turn: &Value) -> Option<&str> {
    turn.pointer("/error/message")
        .and_then(Value::as_str)
        .filter(|reason| !reason.is_empty())
}

fn parent_status(thread: &Value) -> ParentStatus {
    let status = thread
        .pointer("/thread/status/type")
        .and_then(Value::as_str);
    match status {
        Some("idle") => ParentStatus::Idle,
        Some("active") => ParentStatus::Active,
        Some("notLoaded") => ParentStatus::NotLoaded,
        Some("systemError") => ParentStatus::SystemError,
        Some(status) => ParentStatus::Unknown(Some(status.to_string())),
        None => ParentStatus::Unknown(None),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::{HashSet, VecDeque};
    use std::rc::Rc;
    use std::time::Duration;

    use super::*;
    use anyhow::anyhow;
    use indoc::formatdoc;
    use rstest::{fixture, rstest};
    use serde_json::json;

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum ApiCall {
        Read(String, bool),
        Queue(String, String),
    }

    #[derive(Debug, PartialEq, Eq)]
    struct Observation {
        outcome: std::result::Result<Outcome, String>,
        calls: Vec<ApiCall>,
    }

    #[derive(Default)]
    struct FakeControl {
        responses: VecDeque<std::result::Result<Value, String>>,
        calls: Vec<ApiCall>,
        queue_error: Option<String>,
    }

    impl FakeControl {
        fn with_responses(
            responses: impl IntoIterator<Item = std::result::Result<Value, String>>,
        ) -> Self {
            Self {
                responses: responses.into_iter().collect(),
                calls: Vec::new(),
                queue_error: None,
            }
        }
    }

    impl WakeParentControl for FakeControl {
        fn read_thread(&mut self, thread_id: &str, include_turns: bool) -> Result<Value> {
            self.calls
                .push(ApiCall::Read(thread_id.to_string(), include_turns));
            self.responses
                .pop_front()
                .unwrap_or_else(|| Err("unexpected thread/read".to_string()))
                .map_err(|error| anyhow!(error))
        }

        fn queue_add(&mut self, thread_id: &str, content: &str) -> Result<()> {
            self.calls
                .push(ApiCall::Queue(thread_id.to_string(), content.to_string()));
            match &self.queue_error {
                Some(error) => Err(anyhow!(error.clone())),
                None => Ok(()),
            }
        }
    }

    #[derive(Default)]
    struct MemoryLedger(Rc<RefCell<MemoryLedgerState>>);

    #[derive(Default)]
    struct MemoryLedgerState {
        pending: HashSet<EventKey>,
        processed: HashSet<EventKey>,
    }

    impl EventLedger for MemoryLedger {
        fn begin(&self, key: &EventKey) -> Result<Option<Box<dyn EventLease>>> {
            let mut state = self.0.borrow_mut();
            if state.processed.contains(key) || !state.pending.insert(key.clone()) {
                return Ok(None);
            }
            Ok(Some(Box::new(MemoryLease {
                state: Rc::clone(&self.0),
                key: key.clone(),
                marked: false,
            })))
        }
    }

    struct MemoryLease {
        state: Rc<RefCell<MemoryLedgerState>>,
        key: EventKey,
        marked: bool,
    }

    impl EventLease for MemoryLease {
        fn mark_processed(&mut self) -> Result<()> {
            let mut state = self.state.borrow_mut();
            state.pending.remove(&self.key);
            state.processed.insert(self.key.clone());
            self.marked = true;
            Ok(())
        }
    }

    impl Drop for MemoryLease {
        fn drop(&mut self) {
            if !self.marked {
                self.state.borrow_mut().pending.remove(&self.key);
            }
        }
    }

    #[fixture]
    fn input() -> HookInput {
        HookInput {
            session_id: "sample-root-session".to_string(),
            agent_id: "sample-child-thread".to_string(),
            turn_id: "sample-child-turn".to_string(),
        }
    }

    #[fixture]
    fn ledger() -> MemoryLedger {
        MemoryLedger::default()
    }

    #[test]
    fn reads_hook_ids_from_stdin_fields() {
        let actual = serde_json::from_value::<HookInput>(json!({
            "session_id": "sample-root-session",
            "agent_id": "sample-child-thread",
            "turn_id": "sample-child-turn",
            "cwd": "/workspace/sample-project",
        }))
        .map_err(|error| error.to_string());

        assert_eq!(
            actual,
            Ok(HookInput {
                session_id: "sample-root-session".to_string(),
                agent_id: "sample-child-thread".to_string(),
                turn_id: "sample-child-turn".to_string(),
            }),
        );
    }

    fn child_thread(turn_status: &str) -> Value {
        let turn = match turn_status {
            "completed" => json!({
                "id": "sample-child-turn",
                "status": turn_status,
                "items": [{
                    "type": "agentMessage",
                    "text": "sample completed result",
                    "phase": "final_answer",
                }],
            }),
            "failed" => json!({
                "id": "sample-child-turn",
                "status": turn_status,
                "error": {"message": "sample child failure reason"},
            }),
            _ => json!({
                "id": "sample-child-turn",
                "status": turn_status,
            }),
        };
        child_thread_with_turn(turn)
    }

    fn child_thread_with_turn(turn: Value) -> Value {
        json!({
            "thread": {
                "parentThreadId": "sample-direct-parent",
                "turns": [turn],
            },
        })
    }

    fn expected_queue_header(status: &str, result_label: &str) -> String {
        let mut header = formatdoc! {"
            Codex subagent turn finished before native mailbox delivery.
            Child thread ID: sample-child-thread
            Turn ID: sample-child-turn
            Status: {status}
            {result_label}:"};
        header.push('\n');
        header
    }

    fn expected_queue_message(status: &str, result_label: &str, result: &str) -> String {
        format!("{}{result}", expected_queue_header(status, result_label))
    }

    fn parent_thread(status: Option<&str>) -> Value {
        match status {
            Some(status) => json!({"thread": {"status": {"type": status}}}),
            None => json!({"thread": {}}),
        }
    }

    fn observe(
        input: &HookInput,
        control: &mut FakeControl,
        ledger: &MemoryLedger,
        max_reads: usize,
    ) -> Observation {
        let outcome = process_event(input, control, ledger, max_reads, Duration::ZERO, |_| {})
            .map_err(|error| error.to_string());
        Observation {
            outcome,
            calls: std::mem::take(&mut control.calls),
        }
    }

    #[rstest]
    #[case::completed("completed")]
    #[case::failed("failed")]
    fn queues_the_terminal_child_turn_result_for_the_direct_idle_parent(
        input: HookInput,
        ledger: MemoryLedger,
        #[case] terminal_status: &str,
    ) {
        let mut control = FakeControl::with_responses([
            Ok(child_thread("inProgress")),
            Ok(child_thread(terminal_status)),
            Ok(parent_thread(Some("idle"))),
        ]);
        let actual = observe(&input, &mut control, &ledger, 3);
        let expected_message = match terminal_status {
            "completed" => {
                expected_queue_message("completed", "Final answer", "sample completed result")
            }
            "failed" => {
                expected_queue_message("failed", "Failure reason", "sample child failure reason")
            }
            _ => unreachable!(),
        };

        assert_eq!(
            actual,
            Observation {
                outcome: Ok(Outcome::Queued),
                calls: vec![
                    ApiCall::Read("sample-child-thread".to_string(), true),
                    ApiCall::Read("sample-child-thread".to_string(), true),
                    ApiCall::Read("sample-direct-parent".to_string(), false),
                    ApiCall::Queue("sample-direct-parent".to_string(), expected_message,),
                ],
            },
        );
    }

    #[rstest]
    #[case::completed_without_final_answer(
        "completed",
        None,
        "Final answer",
        "No final answer was available in the child turn."
    )]
    #[case::failed_without_error(
        "failed",
        None,
        "Failure reason",
        "No failure reason was provided in the child turn."
    )]
    #[case::failed_with_empty_error(
        "failed",
        Some(""),
        "Failure reason",
        "No failure reason was provided in the child turn."
    )]
    fn queues_fallback_text_when_the_terminal_turn_has_no_result(
        input: HookInput,
        ledger: MemoryLedger,
        #[case] terminal_status: &str,
        #[case] error_message: Option<&str>,
        #[case] result_label: &str,
        #[case] expected_result: &str,
    ) {
        let mut turn = json!({
            "id": "sample-child-turn",
            "status": terminal_status,
        });
        if terminal_status == "completed" {
            turn["items"] = json!([{
                "type": "agentMessage",
                "text": "sample commentary",
                "phase": "commentary",
            }]);
        }
        if let Some(error_message) = error_message {
            turn["error"] = json!({"message": error_message});
        }
        let mut control = FakeControl::with_responses([
            Ok(child_thread_with_turn(turn)),
            Ok(parent_thread(Some("idle"))),
        ]);
        let actual = observe(&input, &mut control, &ledger, 1);

        assert_eq!(
            actual,
            Observation {
                outcome: Ok(Outcome::Queued),
                calls: vec![
                    ApiCall::Read("sample-child-thread".to_string(), true),
                    ApiCall::Read("sample-direct-parent".to_string(), false),
                    ApiCall::Queue(
                        "sample-direct-parent".to_string(),
                        expected_queue_message(terminal_status, result_label, expected_result,),
                    ),
                ],
            },
        );
    }

    #[rstest]
    fn queues_the_child_turn_result_for_an_active_parent_without_steering_its_turn(
        input: HookInput,
        ledger: MemoryLedger,
    ) {
        let mut control = FakeControl::with_responses([
            Ok(child_thread("completed")),
            Ok(parent_thread(Some("active"))),
        ]);
        let actual = observe(&input, &mut control, &ledger, 1);

        assert_eq!(
            actual,
            Observation {
                outcome: Ok(Outcome::Queued),
                calls: vec![
                    ApiCall::Read("sample-child-thread".to_string(), true),
                    ApiCall::Read("sample-direct-parent".to_string(), false),
                    ApiCall::Queue(
                        "sample-direct-parent".to_string(),
                        expected_queue_message(
                            "completed",
                            "Final answer",
                            "sample completed result",
                        ),
                    ),
                ],
            },
        );
    }

    #[rstest]
    #[case::final_answer_is_preferred("final_answer")]
    #[case::missing_phase_uses_last_agent_message("missing_phase")]
    fn selects_the_final_answer_or_last_unphased_agent_message(
        input: HookInput,
        ledger: MemoryLedger,
        #[case] scenario: &str,
    ) {
        let (items, expected_result) = match scenario {
            "final_answer" => (
                json!([
                    {
                        "type": "agentMessage",
                        "text": "sample final answer",
                        "phase": "final_answer",
                    },
                    {
                        "type": "agentMessage",
                        "text": "sample later unphased message",
                    },
                ]),
                "sample final answer",
            ),
            "missing_phase" => (
                json!([
                    {
                        "type": "agentMessage",
                        "text": "sample commentary",
                        "phase": "commentary",
                    },
                    {
                        "type": "agentMessage",
                        "text": "sample latest legacy result",
                    },
                ]),
                "sample latest legacy result",
            ),
            _ => unreachable!(),
        };
        let turn = json!({
            "id": "sample-child-turn",
            "status": "completed",
            "items": items,
        });
        let mut control = FakeControl::with_responses([
            Ok(child_thread_with_turn(turn)),
            Ok(parent_thread(Some("idle"))),
        ]);
        let actual = observe(&input, &mut control, &ledger, 1);

        assert_eq!(
            actual,
            Observation {
                outcome: Ok(Outcome::Queued),
                calls: vec![
                    ApiCall::Read("sample-child-thread".to_string(), true),
                    ApiCall::Read("sample-direct-parent".to_string(), false),
                    ApiCall::Queue(
                        "sample-direct-parent".to_string(),
                        expected_queue_message("completed", "Final answer", expected_result),
                    ),
                ],
            },
        );
    }

    #[rstest]
    fn truncates_long_results_to_the_queue_limit_and_keeps_child_references(
        input: HookInput,
        ledger: MemoryLedger,
    ) {
        let result = "木".repeat(MAX_QUEUE_MESSAGE_CHARS);
        let turn = json!({
            "id": "sample-child-turn",
            "status": "completed",
            "items": [{
                "type": "agentMessage",
                "text": result,
                "phase": "final_answer",
            }],
        });
        let mut control = FakeControl::with_responses([
            Ok(child_thread_with_turn(turn)),
            Ok(parent_thread(Some("idle"))),
        ]);
        let actual = observe(&input, &mut control, &ledger, 1);
        let header = expected_queue_header("completed", "Final answer");
        let result_budget = MAX_QUEUE_MESSAGE_CHARS
            - header.chars().count()
            - QUEUE_MESSAGE_TRUNCATION_NOTICE.chars().count()
            - 2;
        let mut expected_message = format!("{header}{}", "木".repeat(result_budget));
        expected_message.push('\n');
        expected_message.push('\n');
        expected_message.push_str(QUEUE_MESSAGE_TRUNCATION_NOTICE);

        assert_eq!(
            actual,
            Observation {
                outcome: Ok(Outcome::Queued),
                calls: vec![
                    ApiCall::Read("sample-child-thread".to_string(), true),
                    ApiCall::Read("sample-direct-parent".to_string(), false),
                    ApiCall::Queue("sample-direct-parent".to_string(), expected_message),
                ],
            },
        );
    }

    #[rstest]
    fn does_not_queue_or_retry_after_child_turn_is_interrupted(
        input: HookInput,
        ledger: MemoryLedger,
    ) {
        let mut control = FakeControl::with_responses([Ok(child_thread("interrupted"))]);

        let actual = (
            observe(&input, &mut control, &ledger, 1),
            observe(&input, &mut control, &ledger, 1),
        );

        assert_eq!(
            actual,
            (
                Observation {
                    outcome: Ok(Outcome::ChildTurnInterrupted),
                    calls: vec![ApiCall::Read("sample-child-thread".to_string(), true)],
                },
                Observation {
                    outcome: Ok(Outcome::Duplicate),
                    calls: vec![],
                },
            ),
        );
    }

    #[rstest]
    fn does_not_treat_another_completed_turn_as_the_target_turn(
        input: HookInput,
        ledger: MemoryLedger,
    ) {
        let mut control = FakeControl::with_responses([
            Ok(json!({
                "thread": {
                    "parentThreadId": "sample-direct-parent",
                    "turns": [
                        {"id": "sample-other-turn", "status": "completed"},
                        {"id": "sample-child-turn", "status": "inProgress"},
                    ],
                },
            })),
            Ok(child_thread("completed")),
            Ok(parent_thread(Some("idle"))),
        ]);
        let actual = observe(&input, &mut control, &ledger, 3);

        assert_eq!(
            actual,
            Observation {
                outcome: Ok(Outcome::Queued),
                calls: vec![
                    ApiCall::Read("sample-child-thread".to_string(), true),
                    ApiCall::Read("sample-child-thread".to_string(), true),
                    ApiCall::Read("sample-direct-parent".to_string(), false),
                    ApiCall::Queue(
                        "sample-direct-parent".to_string(),
                        expected_queue_message(
                            "completed",
                            "Final answer",
                            "sample completed result",
                        ),
                    ),
                ],
            },
        );
    }

    #[rstest]
    #[case::not_loaded("notLoaded", ParentStatus::NotLoaded)]
    #[case::system_error("systemError", ParentStatus::SystemError)]
    #[case::unknown("futureState", ParentStatus::Unknown(Some("futureState".to_string())))]
    fn does_not_queue_when_parent_status_is_not_idle_or_active(
        input: HookInput,
        ledger: MemoryLedger,
        #[case] status: &str,
        #[case] expected: ParentStatus,
    ) {
        let mut control = FakeControl::with_responses([
            Ok(child_thread("completed")),
            Ok(parent_thread(Some(status))),
        ]);
        let actual = observe(&input, &mut control, &ledger, 1);

        assert_eq!(
            actual,
            Observation {
                outcome: Ok(Outcome::ParentNotQueueable(expected)),
                calls: vec![
                    ApiCall::Read("sample-child-thread".to_string(), true),
                    ApiCall::Read("sample-direct-parent".to_string(), false),
                ],
            },
        );
    }

    #[rstest]
    fn does_not_queue_when_parent_status_is_missing(input: HookInput, ledger: MemoryLedger) {
        let mut control =
            FakeControl::with_responses([Ok(child_thread("completed")), Ok(parent_thread(None))]);

        let actual = observe(&input, &mut control, &ledger, 1);

        assert_eq!(
            actual,
            Observation {
                outcome: Ok(Outcome::ParentNotQueueable(ParentStatus::Unknown(None))),
                calls: vec![
                    ApiCall::Read("sample-child-thread".to_string(), true),
                    ApiCall::Read("sample-direct-parent".to_string(), false),
                ],
            },
        );
    }

    #[rstest]
    fn does_not_queue_when_the_child_turn_is_still_in_progress(
        input: HookInput,
        ledger: MemoryLedger,
    ) {
        let mut control = FakeControl::with_responses([
            Ok(child_thread("inProgress")),
            Ok(child_thread("inProgress")),
        ]);
        let actual = observe(&input, &mut control, &ledger, 2);

        assert_eq!(
            actual,
            Observation {
                outcome: Ok(Outcome::ChildTurnPending),
                calls: vec![
                    ApiCall::Read("sample-child-thread".to_string(), true),
                    ApiCall::Read("sample-child-thread".to_string(), true),
                ],
            },
        );
    }

    #[rstest]
    fn does_not_queue_when_thread_state_cannot_be_read(input: HookInput, ledger: MemoryLedger) {
        let mut control =
            FakeControl::with_responses([Err("control socket unavailable".to_string())]);

        let actual = observe(&input, &mut control, &ledger, 1);

        assert_eq!(
            actual,
            Observation {
                outcome: Err("failed to read child thread sample-child-thread".to_string()),
                calls: vec![ApiCall::Read("sample-child-thread".to_string(), true)],
            },
        );
    }

    #[rstest]
    fn does_not_queue_when_the_parent_thread_cannot_be_read(
        input: HookInput,
        ledger: MemoryLedger,
    ) {
        let mut control = FakeControl::with_responses([
            Ok(child_thread("completed")),
            Err("parent thread is unavailable".to_string()),
        ]);
        let actual = observe(&input, &mut control, &ledger, 1);

        assert_eq!(
            actual,
            Observation {
                outcome: Ok(Outcome::ParentUnavailable),
                calls: vec![
                    ApiCall::Read("sample-child-thread".to_string(), true),
                    ApiCall::Read("sample-direct-parent".to_string(), false),
                ],
            },
        );
    }

    #[rstest]
    #[case::idle("idle")]
    #[case::active("active")]
    fn queues_a_child_turn_result_only_once_for_duplicate_hooks(
        input: HookInput,
        ledger: MemoryLedger,
        #[case] parent_status: &str,
    ) {
        let mut control = FakeControl::with_responses([
            Ok(child_thread("completed")),
            Ok(parent_thread(Some(parent_status))),
        ]);
        let first = observe(&input, &mut control, &ledger, 1);
        let second = observe(&input, &mut control, &ledger, 1);

        assert_eq!(
            (first, second),
            (
                Observation {
                    outcome: Ok(Outcome::Queued),
                    calls: vec![
                        ApiCall::Read("sample-child-thread".to_string(), true),
                        ApiCall::Read("sample-direct-parent".to_string(), false),
                        ApiCall::Queue(
                            "sample-direct-parent".to_string(),
                            expected_queue_message(
                                "completed",
                                "Final answer",
                                "sample completed result",
                            ),
                        ),
                    ],
                },
                Observation {
                    outcome: Ok(Outcome::Duplicate),
                    calls: Vec::new(),
                },
            ),
        );
    }

    #[rstest]
    fn records_a_rejected_queue_attempt_without_retrying(input: HookInput, ledger: MemoryLedger) {
        let mut control = FakeControl::with_responses([
            Ok(child_thread("completed")),
            Ok(parent_thread(Some("idle"))),
        ]);
        control.queue_error = Some("queue rejected".to_string());
        let first = observe(&input, &mut control, &ledger, 1);
        let second = observe(&input, &mut control, &ledger, 1);

        assert_eq!(
            (first, second),
            (
                Observation {
                    outcome: Ok(Outcome::QueueRejected),
                    calls: vec![
                        ApiCall::Read("sample-child-thread".to_string(), true),
                        ApiCall::Read("sample-direct-parent".to_string(), false),
                        ApiCall::Queue(
                            "sample-direct-parent".to_string(),
                            expected_queue_message(
                                "completed",
                                "Final answer",
                                "sample completed result",
                            ),
                        ),
                    ],
                },
                Observation {
                    outcome: Ok(Outcome::Duplicate),
                    calls: Vec::new(),
                },
            ),
        );
    }
}
