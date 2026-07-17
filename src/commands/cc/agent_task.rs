//! Liveness probe for Claude Code Task-tool (Agent) background subagents.
//!
//! Mirrors `bg_task.rs`'s approach for Bash `run_in_background` tasks, but
//! for subagents launched via the Task tool the same way. This codebase does
//! not wire `SubagentStop` or read the `background_tasks` array Claude Code
//! v2.1.145+ exposes on `Stop`/`SubagentStop` input (either of which could
//! report completion directly) -- doing so would raise the minimum
//! supported Claude Code version and is left for a follow-up. So removal
//! here is driven purely by this liveness probe rather than by any
//! hook-based signal. Claude Code holds the subagent's output file open for
//! writing until it exits, exactly like a Bash bg task, so the same
//! `lsof -t` technique applies -- the only difference is the file path is
//! given directly by `tool_response.outputFile` rather than reconstructed
//! from an id.

use std::path::Path;

use super::bg_task::file_is_held_open;
use super::types::Session;

pub trait AgentTaskProbe {
    fn is_alive(&self, output_file: &Path) -> bool;
}

pub struct LsofAgentTaskProbe;

impl AgentTaskProbe for LsofAgentTaskProbe {
    fn is_alive(&self, output_file: &Path) -> bool {
        file_is_held_open(output_file)
    }
}

/// Removes dead output file paths from `session.pending_agent_task_outputs`.
/// Returns `true` when at least one path was removed (caller is expected to
/// persist the session in that case).
pub fn sweep_pending_agent_tasks<P: AgentTaskProbe>(session: &mut Session, probe: &P) -> bool {
    if session.pending_agent_task_outputs.is_empty() {
        return false;
    }
    let mut changed = false;
    session.pending_agent_task_outputs.retain(|path| {
        let alive = probe.is_alive(path);
        if !alive {
            changed = true;
        }
        alive
    });
    changed
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::cell::RefCell;
    use std::collections::HashSet;
    use std::path::PathBuf;

    use super::*;

    /// Test double that reports a fixed set of output file paths as "alive".
    /// Any path not in the set is treated as dead, matching the production
    /// lsof behavior on a subagent that has already exited.
    #[derive(Default)]
    pub(crate) struct FakeAgentTaskProbe {
        alive: RefCell<HashSet<PathBuf>>,
    }

    impl FakeAgentTaskProbe {
        pub(crate) fn with_alive(paths: &[&str]) -> Self {
            let probe = Self::default();
            for path in paths {
                probe.alive.borrow_mut().insert(PathBuf::from(path));
            }
            probe
        }
    }

    impl AgentTaskProbe for FakeAgentTaskProbe {
        fn is_alive(&self, output_file: &Path) -> bool {
            self.alive.borrow().contains(output_file)
        }
    }

    /// Probe that treats every output file as dead. Useful for asserting
    /// that the sweep removes everything.
    pub(crate) struct AllDeadAgentTaskProbe;

    impl AgentTaskProbe for AllDeadAgentTaskProbe {
        fn is_alive(&self, _output_file: &Path) -> bool {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use chrono::Utc;
    use rstest::rstest;

    use super::test_support::{AllDeadAgentTaskProbe, FakeAgentTaskProbe};
    use super::*;
    use crate::commands::cc::types::SessionStatus;

    fn make_session(output_files: &[&str]) -> Session {
        let now = Utc::now();
        Session {
            session_id: "sess".to_string(),
            cwd: PathBuf::from("/tmp/wt"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Stopped,
            created_at: now,
            updated_at: now,
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: BTreeSet::new(),
            pending_agent_task_outputs: output_files.iter().map(PathBuf::from).collect(),
            read_at: None,
            sweep_signaled: false,
        }
    }

    #[rstest]
    fn sweep_is_noop_when_no_pending() {
        let mut session = make_session(&[]);
        let probe = AllDeadAgentTaskProbe;
        assert!(!sweep_pending_agent_tasks(&mut session, &probe));
        assert!(session.pending_agent_task_outputs.is_empty());
    }

    #[rstest]
    fn sweep_removes_all_dead_output_files() {
        let mut session = make_session(&["/tmp/a.output", "/tmp/b.output"]);
        let probe = AllDeadAgentTaskProbe;
        assert!(sweep_pending_agent_tasks(&mut session, &probe));
        assert!(session.pending_agent_task_outputs.is_empty());
    }

    #[rstest]
    fn sweep_retains_alive_output_files() {
        let mut session = make_session(&["/tmp/a.output", "/tmp/b.output", "/tmp/c.output"]);
        let probe = FakeAgentTaskProbe::with_alive(&["/tmp/b.output"]);
        assert!(sweep_pending_agent_tasks(&mut session, &probe));
        let remaining: BTreeSet<PathBuf> =
            session.pending_agent_task_outputs.iter().cloned().collect();
        assert_eq!(remaining, BTreeSet::from([PathBuf::from("/tmp/b.output")]));
    }

    #[rstest]
    fn sweep_returns_false_when_nothing_changed() {
        let mut session = make_session(&["/tmp/a.output", "/tmp/b.output"]);
        let probe = FakeAgentTaskProbe::with_alive(&["/tmp/a.output", "/tmp/b.output"]);
        assert!(!sweep_pending_agent_tasks(&mut session, &probe));
        assert_eq!(session.pending_agent_task_outputs.len(), 2);
    }
}
