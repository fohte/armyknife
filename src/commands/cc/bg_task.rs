//! Liveness probe for Claude Code background task ids.
//!
//! Claude Code holds the per-bg-task output file open for writing until the
//! task exits, so `lsof -t -- <file>` returning a pid means the writer is
//! still alive. Probe is fail-open: any ambiguity (missing file, lsof error)
//! is treated as alive so a live task is never dropped from the pending set.
use std::path::{Path, PathBuf};
use std::process::Command;

use super::types::Session;

pub trait BgTaskProbe {
    fn is_alive(&self, cwd: &Path, session_id: &str, bg_id: &str) -> bool;
}

/// Mirrors Claude Code's own `~/.claude/projects/<dir>` naming scheme. Probe
/// correctness depends on staying in sync with that scheme; if it drifts the
/// probe will look at the wrong file and always report alive.
pub fn encode_project_dir(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c == '/' || c == '.' { '-' } else { c })
        .collect()
}

pub fn bg_task_output_path(cwd: &Path, session_id: &str, bg_id: &str) -> PathBuf {
    // SAFETY: getuid() has no preconditions and is documented as always
    // succeeding on POSIX systems.
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/tmp/claude-{uid}"))
        .join(encode_project_dir(cwd))
        .join(session_id)
        .join("tasks")
        .join(format!("{bg_id}.output"))
}

pub struct LsofBgTaskProbe;

impl BgTaskProbe for LsofBgTaskProbe {
    fn is_alive(&self, cwd: &Path, session_id: &str, bg_id: &str) -> bool {
        let path = bg_task_output_path(cwd, session_id, bg_id);
        if !path.exists() {
            // fail-open: the output file may not have been created yet in
            // the brief window between PostToolUse hook and Claude Code
            // opening the file. Treat absence as alive so a still-running
            // task is never dropped.
            return true;
        }
        match Command::new("lsof").arg("-t").arg("--").arg(&path).output() {
            Ok(out) => out.status.success() && !out.stdout.trim_ascii().is_empty(),
            // fail-open on lsof error so a transient failure does not drop
            // a live bg id.
            Err(_) => true,
        }
    }
}

/// Removes dead bg ids from `session.pending_bg_task_ids`. Returns `true`
/// when at least one id was removed (caller is expected to persist the
/// session in that case).
pub fn sweep_pending_bg_tasks<P: BgTaskProbe>(session: &mut Session, probe: &P) -> bool {
    if session.pending_bg_task_ids.is_empty() {
        return false;
    }
    // Destructure so the retain closure borrows pending_bg_task_ids mutably
    // and cwd / session_id immutably without conflict.
    let Session {
        pending_bg_task_ids,
        cwd,
        session_id,
        ..
    } = session;
    let mut changed = false;
    pending_bg_task_ids.retain(|bg| {
        let alive = probe.is_alive(cwd, session_id, bg);
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

    use super::*;

    /// Test double that reports a fixed set of bg ids as "alive". Any bg id
    /// not in the set is treated as dead, matching the production lsof
    /// behavior on a process that has already exited.
    #[derive(Default)]
    pub(crate) struct FakeBgTaskProbe {
        alive: RefCell<HashSet<String>>,
    }

    impl FakeBgTaskProbe {
        pub(crate) fn with_alive(ids: &[&str]) -> Self {
            let probe = Self::default();
            for id in ids {
                probe.alive.borrow_mut().insert((*id).to_string());
            }
            probe
        }
    }

    impl BgTaskProbe for FakeBgTaskProbe {
        fn is_alive(&self, _cwd: &Path, _session_id: &str, bg_id: &str) -> bool {
            self.alive.borrow().contains(bg_id)
        }
    }

    /// Probe that treats every bg id as dead. Useful for asserting that the
    /// sweep removes everything.
    pub(crate) struct AllDeadBgTaskProbe;

    impl BgTaskProbe for AllDeadBgTaskProbe {
        fn is_alive(&self, _cwd: &Path, _session_id: &str, _bg_id: &str) -> bool {
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

    use super::test_support::{AllDeadBgTaskProbe, FakeBgTaskProbe};
    use super::*;
    use crate::commands::cc::types::SessionStatus;

    fn make_session(bg_ids: &[&str]) -> Session {
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
            pending_bg_task_ids: bg_ids.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[rstest]
    #[case::plain_path(
        "/Users/fohte/ghq/github.com/fohte/armyknife",
        "-Users-fohte-ghq-github-com-fohte-armyknife"
    )]
    #[case::with_worktree(
        "/Users/fohte/ghq/github.com/fohte/armyknife/.worktrees/branch",
        "-Users-fohte-ghq-github-com-fohte-armyknife--worktrees-branch"
    )]
    #[case::dotfile_in_path("/home/u/.config/app", "-home-u--config-app")]
    fn encode_project_dir_matches_claude_code_scheme(#[case] input: &str, #[case] expected: &str) {
        assert_eq!(encode_project_dir(Path::new(input)), expected);
    }

    #[rstest]
    fn bg_task_output_path_includes_encoded_cwd_session_and_bg_id() {
        // SAFETY: getuid() has no preconditions on POSIX.
        let uid = unsafe { libc::getuid() };
        let path = bg_task_output_path(Path::new("/tmp/wt"), "sess-abc", "bg-xyz");
        let expected = PathBuf::from(format!(
            "/tmp/claude-{uid}/-tmp-wt/sess-abc/tasks/bg-xyz.output"
        ));
        assert_eq!(path, expected);
    }

    #[rstest]
    fn sweep_is_noop_when_no_pending() {
        let mut session = make_session(&[]);
        let probe = AllDeadBgTaskProbe;
        assert!(!sweep_pending_bg_tasks(&mut session, &probe));
        assert!(session.pending_bg_task_ids.is_empty());
    }

    #[rstest]
    fn sweep_removes_all_dead_bg_ids() {
        let mut session = make_session(&["bg-1", "bg-2", "bg-3"]);
        let probe = AllDeadBgTaskProbe;
        assert!(sweep_pending_bg_tasks(&mut session, &probe));
        assert!(session.pending_bg_task_ids.is_empty());
    }

    #[rstest]
    fn sweep_retains_alive_bg_ids() {
        let mut session = make_session(&["bg-1", "bg-2", "bg-3"]);
        let probe = FakeBgTaskProbe::with_alive(&["bg-2"]);
        assert!(sweep_pending_bg_tasks(&mut session, &probe));
        let remaining: BTreeSet<&str> = session
            .pending_bg_task_ids
            .iter()
            .map(String::as_str)
            .collect();
        assert_eq!(remaining, BTreeSet::from(["bg-2"]));
    }

    #[rstest]
    fn sweep_returns_false_when_nothing_changed() {
        let mut session = make_session(&["bg-1", "bg-2"]);
        let probe = FakeBgTaskProbe::with_alive(&["bg-1", "bg-2"]);
        assert!(!sweep_pending_bg_tasks(&mut session, &probe));
        assert_eq!(session.pending_bg_task_ids.len(), 2);
    }
}
