use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::{ProcessSnapshot, list_process_cwds};

pub const SHELL_COMMANDS: &[&str] = &["zsh", "bash", "fish", "sh", "dash"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInPath {
    pub pid: u32,
    pub comm: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct WorktreeProcessInspection {
    pub(crate) non_shell_processes: Vec<ProcessInPath>,
    pub(crate) daemon_descendant_pgids: Vec<libc::pid_t>,
}

/// Finds non-shell processes whose current working directory is inside `path`.
///
/// Returns `None` when either process source cannot be inspected, so callers
/// can refuse destructive work when they cannot verify the worktree is idle.
pub fn find_non_shell_processes_in_path(path: &Path) -> Option<Vec<ProcessInPath>> {
    inspect_worktree_processes(path).map(|inspection| inspection.non_shell_processes)
}

pub(crate) fn inspect_worktree_processes(path: &Path) -> Option<WorktreeProcessInspection> {
    let cwds = list_process_cwds()?;
    let snapshot = ProcessSnapshot::capture_with_command_lines_and_groups()?;
    let resolved_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    Some(inspect_processes_in_path(
        &snapshot,
        &cwds,
        std::process::id(),
        &resolved_path,
    ))
}

fn inspect_processes_in_path(
    snapshot: &ProcessSnapshot,
    cwds: &HashMap<u32, PathBuf>,
    self_pid: u32,
    path: &Path,
) -> WorktreeProcessInspection {
    let excluded_pids = snapshot.ancestors(self_pid);
    let candidates = cwds
        .iter()
        .filter(|(pid, cwd)| !excluded_pids.contains(pid) && cwd.starts_with(path))
        .map(|(&pid, _)| {
            (
                pid,
                snapshot.codex_app_server_ancestors(pid).collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    let daemon_pids = candidates
        .iter()
        .flat_map(|(_, ancestors)| ancestors.iter().copied())
        .collect::<HashSet<_>>();
    let excluded_pgids = excluded_pids
        .iter()
        .chain(daemon_pids.iter())
        .filter_map(|&pid| snapshot.pgid(pid))
        .collect::<HashSet<_>>();

    let mut non_shell_processes = Vec::new();
    let mut daemon_descendant_pgids = HashSet::new();
    for (pid, daemon_ancestors) in candidates {
        if !daemon_ancestors.is_empty() {
            if let Some(pgid) = snapshot.pgid(pid)
                && pgid > 1
                && !excluded_pgids.contains(&pgid)
            {
                daemon_descendant_pgids.insert(pgid);
            }
            continue;
        }
        let Some(comm) = snapshot.comm_basename(pid) else {
            continue;
        };
        let comm = comm.strip_prefix('-').unwrap_or(comm);
        if !SHELL_COMMANDS.contains(&comm) && !is_shell_histdb_sqlite3(snapshot, pid, comm) {
            non_shell_processes.push(ProcessInPath {
                pid,
                comm: comm.to_string(),
            });
        }
    }
    non_shell_processes.sort_by_key(|process| process.pid);
    let mut daemon_descendant_pgids = daemon_descendant_pgids.into_iter().collect::<Vec<_>>();
    daemon_descendant_pgids.sort_unstable();
    WorktreeProcessInspection {
        non_shell_processes,
        daemon_descendant_pgids,
    }
}

fn is_shell_histdb_sqlite3(snapshot: &ProcessSnapshot, pid: u32, comm: &str) -> bool {
    if comm != "sqlite3" {
        return false;
    }

    let Some(parent_pid) = snapshot.parent_pid(pid) else {
        return false;
    };
    let Some(parent_comm) = snapshot.comm_basename(parent_pid) else {
        return false;
    };
    let parent_comm = parent_comm.strip_prefix('-').unwrap_or(parent_comm);
    if !SHELL_COMMANDS.contains(&parent_comm) {
        return false;
    }

    let Some(command_line) = snapshot.command_line(pid) else {
        return false;
    };
    let mut args = command_line.split_whitespace();
    let Some(_executable) = args.next() else {
        return false;
    };
    args.next() == Some("-batch") && args.next() == Some("-noheader")
}

#[cfg(test)]
mod tests {
    use indoc::indoc;
    use rstest::rstest;

    use super::*;

    const WORKTREE: &str = "/repo/.worktrees/feature";

    fn snapshot() -> ProcessSnapshot {
        ProcessSnapshot::from_ps_outputs_with_groups(
            indoc! {"
            1 0 1 launchd
            10 1 10 zsh
            20 10 20 -zsh
            30 20 30 claude
            40 30 40 node
            50 1 50 bash
            60 1 60 /opt/homebrew/bin/codex
            61 60 61 node
            62 60 60 node
            63 60 20 node
            64 60 1 node
            70 1 70 /opt/homebrew/bin/codex
            71 70 71 node
            80 1 80 /usr/bin/bash
            90 1 90 /usr/local/bin/codex
            91 90 91 worker
            100 20 20 sqlite3
            101 20 20 sqlite3
            110 1 110 sqlite3
        "},
            indoc! {"
            60 /opt/homebrew/bin/codex app-server --listen unix:// --managed-daemon
            61 node node_repl
            62 node node_repl
            63 node node_repl
            64 node node_repl
            70 /opt/homebrew/bin/codex exec
            71 node node_repl
            90 /usr/local/bin/codex app-server --listen socket
            91 worker worker
            100 sqlite3 -batch -noheader /Users/example/.histdb/history.db
            101 sqlite3 /Users/example/.histdb/history.db
            110 sqlite3 -batch -noheader /Users/example/.histdb/history.db
        "},
        )
    }

    #[rstest]
    #[case::only_shells(&[(20, WORKTREE), (50, WORKTREE)], &[], &[])]
    #[case::non_shell_processes(&[(30, WORKTREE), (40, WORKTREE)], &[(30, "claude"), (40, "node")], &[])]
    #[case::shell_histdb_sqlite3_is_ignored(&[(100, WORKTREE)], &[], &[])]
    #[case::shell_agent_is_still_detected(&[(30, WORKTREE)], &[(30, "claude")], &[])]
    #[case::other_shell_sqlite3_is_still_detected(&[(20, WORKTREE), (101, WORKTREE)], &[(101, "sqlite3")], &[])]
    #[case::unrelated_histdb_sqlite3_is_still_detected(&[(110, WORKTREE)], &[(110, "sqlite3")], &[])]
    #[case::caller_and_ancestor_are_excluded(&[(10, WORKTREE), (20, WORKTREE), (30, WORKTREE)], &[(30, "claude")], &[])]
    #[case::missing_ps_pid_is_ignored(&[(92, WORKTREE)], &[], &[])]
    #[case::login_shell_is_recognized(&[(20, WORKTREE)], &[], &[])]
    #[case::command_path_uses_its_basename(&[(80, WORKTREE)], &[], &[])]
    #[case::daemon_process_groups_are_collected(&[(61, WORKTREE), (30, WORKTREE)], &[(30, "claude")], &[61])]
    #[case::other_codex_ancestors_are_not_ignored(&[(71, WORKTREE)], &[(71, "node")], &[])]
    #[case::app_server_without_managed_daemon_does_not_exempt_descendants(&[(91, WORKTREE)], &[(91, "worker")], &[])]
    #[case::app_server_itself_is_not_exempt(&[(60, WORKTREE)], &[(60, "codex")], &[])]
    #[case::app_server_process_group_is_excluded(&[(62, WORKTREE)], &[], &[])]
    #[case::caller_process_group_is_excluded(&[(63, WORKTREE)], &[], &[])]
    #[case::invalid_process_group_is_excluded(&[(64, WORKTREE)], &[], &[])]
    fn selects_non_shell_processes_in_the_worktree(
        #[case] cwd_entries: &[(u32, &str)],
        #[case] expected_processes: &[(u32, &str)],
        #[case] expected_pgids: &[libc::pid_t],
    ) {
        let cwds = cwd_entries
            .iter()
            .map(|&(pid, cwd)| (pid, PathBuf::from(cwd)))
            .collect();

        let actual = inspect_processes_in_path(&snapshot(), &cwds, 20, Path::new(WORKTREE));

        assert_eq!(
            actual,
            WorktreeProcessInspection {
                non_shell_processes: expected_processes
                    .iter()
                    .map(|&(pid, comm)| ProcessInPath {
                        pid,
                        comm: comm.to_string(),
                    })
                    .collect(),
                daemon_descendant_pgids: expected_pgids.to_vec(),
            },
        );
    }
}
