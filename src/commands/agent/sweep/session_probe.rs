use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use super::super::types::{Engine, Session, TmuxInfo};
use crate::infra::process::ProcessSnapshot;
use crate::shared::active_session::{DraftObservation, DraftProbe};

/// Adds sweep-only process signaling and background-run checks to the shared
/// `DraftProbe`.
pub(super) trait SessionProbe: DraftProbe {
    /// Returns every live process hosting `session`. `None` means the process
    /// state could not be determined from the available identity data.
    fn resolve_pids(&self, session: &Session) -> Result<Option<Vec<u32>>>;

    /// A fresh negative result takes precedence over a snapshot that may retain a dead child.
    fn resolve_pids_for_pause(
        &self,
        session: &Session,
        draft_observation: &DraftObservation,
    ) -> Result<Option<Vec<u32>>> {
        if draft_observation.codex_process_running == Some(false) {
            return Ok(Some(Vec::new()));
        }
        self.resolve_pids(session)
    }

    /// Returns whether armyknife is tracking a detached background command
    /// for this session.
    fn has_pending_bg_run(&self, _session: &Session) -> bool {
        false
    }

    /// Returns whether the agent process `pid` has a Claude Code Remote
    /// Control connection, which a pause would take offline.
    fn has_remote_control(&self, _session: &Session, _pid: u32) -> bool {
        false
    }
}

pub(super) struct TmuxSessionProbe<'a, A: DraftProbe> {
    pub(super) snapshot: Option<&'a ProcessSnapshot>,
    pub(super) drafts: A,
    pub(super) tasks_dir: PathBuf,
}

/// Descendant walks are bounded to this many processes. A shell hosting
/// claude has at most a handful of children, so this is a safety cap rather
/// than an expected limit.
const MAX_DESCENDANT_NODES: usize = 64;

impl<A: DraftProbe> DraftProbe for TmuxSessionProbe<'_, A> {
    fn draft_observed_at(&self, session: &Session, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.drafts.draft_observed_at(session, now)
    }

    fn observe_draft(&self, session: &Session, now: DateTime<Utc>) -> DraftObservation {
        self.drafts.observe_draft(session, now)
    }
}

impl<A: DraftProbe> SessionProbe for TmuxSessionProbe<'_, A> {
    fn has_pending_bg_run(&self, session: &Session) -> bool {
        match super::super::bg_tasks::has_pending_in(&self.tasks_dir, &session.session_id) {
            Ok(pending) => pending,
            Err(error) => {
                tracing::warn!(
                    event = "agent.sweep.bg_run_registry_read_failed",
                    session = %session.session_id,
                    error = %error,
                );
                true
            }
        }
    }

    fn has_remote_control(&self, session: &Session, pid: u32) -> bool {
        match super::super::claude_registry::has_remote_control(pid) {
            Ok(connected) => connected,
            Err(error) => {
                tracing::warn!(
                    event = "agent.sweep.remote_control_check_failed",
                    session = %session.session_id,
                    pid = pid,
                    error = %format!("{error:#}"),
                );
                true
            }
        }
    }

    fn resolve_pids(&self, session: &Session) -> Result<Option<Vec<u32>>> {
        let snapshot = self
            .snapshot
            .context("Could not inspect running processes during agent sweep")?;
        resolve_pids_with_lookup(session, snapshot, |tmux_info| {
            Ok(crate::infra::tmux::get_pane_pid_for_socket(
                tmux_info.socket_path.as_deref(),
                &tmux_info.pane_id,
            )?)
        })
    }
}

trait ProcessLookup {
    fn find_pids_by_command_arguments(&self, command: &str, arguments: &[&str]) -> Vec<u32>;
    fn find_agent_in_pane(&self, pane_pid: u32, command: &str) -> Option<u32>;
}

impl ProcessLookup for ProcessSnapshot {
    fn find_pids_by_command_arguments(&self, command: &str, arguments: &[&str]) -> Vec<u32> {
        ProcessSnapshot::find_pids_by_command_arguments(self, command, arguments)
    }

    fn find_agent_in_pane(&self, pane_pid: u32, command: &str) -> Option<u32> {
        self.find_self_or_descendant_by_command(pane_pid, command, MAX_DESCENDANT_NODES)
    }
}

fn resume_arguments(engine: Engine, session_id: &str) -> [&str; 2] {
    match engine {
        Engine::Claude => ["--resume", session_id],
        Engine::Codex => ["resume", session_id],
    }
}

fn resolve_pids_with_lookup<P, F>(
    session: &Session,
    processes: &P,
    pane_pid_lookup: F,
) -> Result<Option<Vec<u32>>>
where
    P: ProcessLookup,
    F: FnOnce(&TmuxInfo) -> Result<Option<u32>>,
{
    let expected_arguments = resume_arguments(session.engine, &session.session_id);
    let mut pids = processes
        .find_pids_by_command_arguments(session.engine.process_name(), &expected_arguments);
    let mut pane_query_error = None;
    if let Some(tmux_info) = session.tmux_info.as_ref() {
        match pane_pid_lookup(tmux_info) {
            Ok(Some(pane_pid)) => {
                if let Some(pid) =
                    processes.find_agent_in_pane(pane_pid, session.engine.process_name())
                {
                    pids.push(pid);
                }
            }
            Ok(None) => {}
            Err(error) => pane_query_error = Some(error),
        }
    }

    pids.sort_unstable();
    pids.dedup();
    if pids.is_empty()
        && let Some(error) = pane_query_error
    {
        return Err(error);
    }
    if pids.is_empty() && session.tmux_info.is_none() && !session.sweep_signaled {
        return Ok(None);
    }
    Ok(Some(pids))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::path::PathBuf;

    use anyhow::anyhow;
    use rstest::rstest;

    use super::*;
    use crate::commands::agent::types::SessionStatus;

    #[derive(Default)]
    struct FakeProcessLookup {
        matching_pids: Vec<u32>,
        pane_agents: HashMap<u32, u32>,
        requested_arguments: RefCell<Option<Vec<String>>>,
    }

    impl ProcessLookup for FakeProcessLookup {
        fn find_pids_by_command_arguments(&self, _command: &str, arguments: &[&str]) -> Vec<u32> {
            *self.requested_arguments.borrow_mut() = Some(
                arguments
                    .iter()
                    .map(|argument| (*argument).to_string())
                    .collect(),
            );
            self.matching_pids.clone()
        }

        fn find_agent_in_pane(&self, pane_pid: u32, _command: &str) -> Option<u32> {
            self.pane_agents.get(&pane_pid).copied()
        }
    }

    fn session(engine: Engine, tmux_info: Option<TmuxInfo>, sweep_signaled: bool) -> Session {
        let now = Utc::now();
        Session {
            launch_id: None,
            archived_at: None,
            pr_context: None,
            created_pr_urls: Vec::new(),
            session_id: "fixture-session".to_string(),
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            child_launch: Default::default(),
            crit_urls: Vec::new(),
            crit_not_waiting_urls: Default::default(),
            web_uis: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: PathBuf::from("/fixture"),
            transcript_path: None,
            tty: None,
            engine,
            tmux_info,
            status: SessionStatus::Paused,
            created_at: now,
            updated_at: now,
            last_prompted_at: None,
            last_message: None,
            current_tool: None,
            current_tool_name: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: Default::default(),
            pending_agent_task_ids: Default::default(),
            pending_permission_agent_ids: Default::default(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled,
            stop_blocked_by_armyknife: false,
            agent_status: None,
            agent_status_clear_pending: false,
            model: None,
            effort: None,
        }
    }

    #[rstest]
    fn resolver_merges_argv_and_pane_processes() {
        let session = session(
            Engine::Claude,
            Some(TmuxInfo {
                session_name: "fixture".to_string(),
                window_name: "fixture".to_string(),
                window_index: 0,
                pane_id: "%1".to_string(),
                socket_path: None,
            }),
            false,
        );
        let processes = FakeProcessLookup {
            matching_pids: vec![301, 201],
            ..FakeProcessLookup::default()
        };
        let mut processes = processes;
        processes.pane_agents.insert(100, 301);

        let pids = resolve_pids_with_lookup(&session, &processes, |_| Ok(Some(100)))
            .expect("resolve")
            .expect("identity is known");

        assert_eq!(
            (pids, processes.requested_arguments.borrow().clone()),
            (
                vec![201, 301],
                Some(vec!["--resume".to_string(), "fixture-session".to_string()]),
            ),
        );
    }

    #[rstest]
    #[case::claude(Engine::Claude, ["--resume", "fixture-session"])]
    #[case::codex(Engine::Codex, ["resume", "fixture-session"])]
    fn resolver_uses_engine_arguments_without_tmux_metadata(
        #[case] engine: Engine,
        #[case] expected_arguments: [&str; 2],
    ) {
        let session = session(engine, None, false);
        let processes = FakeProcessLookup {
            matching_pids: vec![201],
            ..FakeProcessLookup::default()
        };
        let resolved =
            resolve_pids_with_lookup(&session, &processes, |_| Ok(None)).expect("resolve");

        assert_eq!(
            (resolved, processes.requested_arguments.borrow().clone()),
            (
                Some(vec![201]),
                Some(expected_arguments.into_iter().map(str::to_string).collect(),),
            ),
        );
    }

    #[rstest]
    #[case::unmarked_session_cannot_be_resolved(false, None)]
    #[case::marked_session_has_no_live_process(true, Some(Vec::new()))]
    fn resolver_fails_closed_without_tmux_metadata_or_a_matching_process(
        #[case] sweep_signaled: bool,
        #[case] expected: Option<Vec<u32>>,
    ) {
        let session = session(Engine::Claude, None, sweep_signaled);
        let resolved =
            resolve_pids_with_lookup(&session, &FakeProcessLookup::default(), |_| Ok(None))
                .expect("resolve");

        assert_eq!(resolved, expected);
    }

    #[rstest]
    fn resolver_propagates_pane_lookup_failure_without_an_argv_match() {
        let session = session(
            Engine::Claude,
            Some(TmuxInfo {
                session_name: "fixture".to_string(),
                window_name: "fixture".to_string(),
                window_index: 0,
                pane_id: "%1".to_string(),
                socket_path: None,
            }),
            false,
        );

        let result = resolve_pids_with_lookup(&session, &FakeProcessLookup::default(), |_| {
            Err(anyhow!("pane lookup failed"))
        });

        assert_eq!(
            result.err().map(|error| error.to_string()),
            Some("pane lookup failed".to_string())
        );
    }
}
