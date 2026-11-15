use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{ProcessSnapshot, list_process_cwds};

pub const SHELL_COMMANDS: &[&str] = &["zsh", "bash", "fish", "sh", "dash"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInPath {
    pub pid: u32,
    pub comm: String,
}

/// Finds non-shell processes whose current working directory is inside `path`.
///
/// Returns `None` when either process source cannot be inspected, so callers
/// can refuse destructive work when they cannot verify the worktree is idle.
pub fn find_non_shell_processes_in_path(path: &Path) -> Option<Vec<ProcessInPath>> {
    let cwds = list_process_cwds()?;
    let snapshot = ProcessSnapshot::capture()?;
    let resolved_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    Some(non_shell_processes_in_path(
        &snapshot,
        &cwds,
        std::process::id(),
        &resolved_path,
    ))
}

fn non_shell_processes_in_path(
    snapshot: &ProcessSnapshot,
    cwds: &HashMap<u32, PathBuf>,
    self_pid: u32,
    path: &Path,
) -> Vec<ProcessInPath> {
    let excluded_pids = snapshot.ancestors(self_pid);
    let mut processes = cwds
        .iter()
        .filter(|(pid, cwd)| !excluded_pids.contains(pid) && cwd.starts_with(path))
        .filter_map(|(&pid, _)| {
            let comm = snapshot.comm_basename(pid)?;
            let comm = comm.strip_prefix('-').unwrap_or(comm);
            (!SHELL_COMMANDS.contains(&comm)).then(|| ProcessInPath {
                pid,
                comm: comm.to_string(),
            })
        })
        .collect::<Vec<_>>();
    processes.sort_by_key(|process| process.pid);
    processes
}

#[cfg(test)]
mod tests {
    use indoc::indoc;
    use rstest::rstest;

    use super::*;

    const WORKTREE: &str = "/repo/.worktrees/feature";

    fn snapshot() -> ProcessSnapshot {
        ProcessSnapshot::from_ps_output(indoc! {"
            1 0 launchd
            10 1 zsh
            20 10 -zsh
            30 20 claude
            40 30 node
            50 1 bash
            60 1 codex
            70 1 /usr/bin/bash
        "})
    }

    #[rstest]
    #[case::only_shells(&[(20, WORKTREE), (50, WORKTREE)], &[])]
    #[case::non_shell_processes(&[(30, WORKTREE), (40, WORKTREE)], &[(30, "claude"), (40, "node")])]
    #[case::caller_and_ancestor_are_excluded(&[(10, WORKTREE), (20, WORKTREE), (30, WORKTREE)], &[(30, "claude")])]
    #[case::missing_ps_pid_is_ignored(&[(80, WORKTREE)], &[])]
    #[case::login_shell_is_recognized(&[(20, WORKTREE)], &[])]
    #[case::command_path_uses_its_basename(&[(70, WORKTREE)], &[])]
    fn selects_non_shell_processes_in_the_worktree(
        #[case] cwd_entries: &[(u32, &str)],
        #[case] expected_processes: &[(u32, &str)],
    ) {
        let cwds = cwd_entries
            .iter()
            .map(|&(pid, cwd)| (pid, PathBuf::from(cwd)))
            .collect();

        let actual = non_shell_processes_in_path(&snapshot(), &cwds, 20, Path::new(WORKTREE));

        assert_eq!(
            actual,
            expected_processes
                .iter()
                .map(|&(pid, comm)| ProcessInPath {
                    pid,
                    comm: comm.to_string(),
                })
                .collect::<Vec<_>>(),
        );
    }
}
