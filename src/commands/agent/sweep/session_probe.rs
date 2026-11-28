use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use super::super::types::{Engine, Session};
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
        let expected_arguments = match session.engine {
            Engine::Claude => vec!["--resume", session.session_id.as_str()],
            Engine::Codex => vec!["resume", session.session_id.as_str()],
        };
        let mut pids = snapshot
            .find_pids_by_command_arguments(session.engine.process_name(), &expected_arguments);

        let mut pane_query_error = None;
        if let Some(tmux_info) = session.tmux_info.as_ref() {
            match crate::infra::tmux::get_pane_pid_for_socket(
                tmux_info.socket_path.as_deref(),
                &tmux_info.pane_id,
            ) {
                Ok(Some(pane_pid)) => {
                    // When the agent is launched directly as the pane command
                    // (no shell wrapper), pane_pid itself is the agent process.
                    if let Some(pid) = snapshot.find_self_or_descendant_by_command(
                        pane_pid,
                        session.engine.process_name(),
                        MAX_DESCENDANT_NODES,
                    ) {
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
            return Err(error.into());
        }
        if pids.is_empty() && session.tmux_info.is_none() && !session.sweep_signaled {
            return Ok(None);
        }
        Ok(Some(pids))
    }
}
