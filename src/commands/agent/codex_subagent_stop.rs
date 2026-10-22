use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Args;
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
const PARENT_WAKE_MESSAGE: &str =
    "A Codex subagent has completed. Check its result in your agent mailbox and continue the task.";

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
    ParentNotIdle(ParentStatus),
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

    let child_thread =
        match wait_for_child_turn(control, input, max_reads, poll_interval, &mut sleep)? {
            ChildTurn::Completed(thread) => thread,
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
    if parent_status != ParentStatus::Idle {
        lease.mark_processed()?;
        return Ok(Outcome::ParentNotIdle(parent_status));
    }

    // Mark before the request because a lost response cannot reveal whether the
    // queue accepted it, and a duplicate hook must not add a second message.
    lease.mark_processed()?;
    match control.queue_add(parent_thread_id, PARENT_WAKE_MESSAGE) {
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

enum ChildTurn {
    Completed(Value),
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
        let status = thread
            .pointer("/thread/turns")
            .and_then(Value::as_array)
            .and_then(|turns| {
                turns.iter().find_map(|turn| {
                    (turn.get("id").and_then(Value::as_str) == Some(&input.turn_id))
                        .then(|| turn.get("status").and_then(Value::as_str))
                        .flatten()
                })
            });

        match status {
            Some("completed" | "failed") => return Ok(ChildTurn::Completed(thread)),
            Some("interrupted") => return Ok(ChildTurn::Interrupted),
            Some("inProgress") | None => {
                if read_number + 1 < max_reads {
                    sleep(poll_interval);
                }
            }
            Some(status) => return Ok(ChildTurn::Unknown(status.to_string())),
        }
    }
    Ok(ChildTurn::Pending)
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
        json!({
            "thread": {
                "parentThreadId": "sample-direct-parent",
                "turns": [{
                    "id": "sample-child-turn",
                    "status": turn_status,
                }],
            },
        })
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
    fn queues_once_for_the_direct_idle_parent_after_the_child_turn_completes(
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
                        PARENT_WAKE_MESSAGE.to_string(),
                    ),
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
                        PARENT_WAKE_MESSAGE.to_string(),
                    ),
                ],
            },
        );
    }

    #[rstest]
    #[case::active("active", ParentStatus::Active)]
    #[case::not_loaded("notLoaded", ParentStatus::NotLoaded)]
    #[case::system_error("systemError", ParentStatus::SystemError)]
    #[case::unknown("futureState", ParentStatus::Unknown(Some("futureState".to_string())))]
    fn does_not_queue_when_parent_is_not_known_idle(
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
                outcome: Ok(Outcome::ParentNotIdle(expected)),
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
                outcome: Ok(Outcome::ParentNotIdle(ParentStatus::Unknown(None))),
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
    fn does_not_queue_twice_for_a_duplicate_child_turn(input: HookInput, ledger: MemoryLedger) {
        let mut control = FakeControl::with_responses([
            Ok(child_thread("completed")),
            Ok(parent_thread(Some("idle"))),
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
                            PARENT_WAKE_MESSAGE.to_string(),
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
                            PARENT_WAKE_MESSAGE.to_string(),
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
