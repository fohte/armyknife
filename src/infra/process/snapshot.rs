use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::infra::external_tool::ExternalTool;
use crate::shared::command;

pub(crate) fn current_caller_pane_pid() -> Option<u32> {
    let pane_id = crate::shared::caller_pane::resolve_caller_pane_id()?;
    crate::infra::tmux::get_pane_pid(&pane_id)
}

mod caller;
mod worktree;

pub use caller::find_caller_pids_in_path;
pub use worktree::{ProcessInPath, SHELL_COMMANDS, find_non_shell_processes_in_path};
pub(crate) use worktree::{WorktreeProcessInspection, inspect_worktree_processes};

/// Process topology and command names captured from the process table.
///
/// Stores a parent → children mapping so callers can search for descendants
/// without forking `ps` for every lookup.
pub struct ProcessSnapshot {
    children: HashMap<u32, Vec<(u32, String)>>,
    parents: HashMap<u32, u32>,
    comms: HashMap<u32, String>,
    command_lines: HashMap<u32, String>,
    pgids: HashMap<u32, libc::pid_t>,
}

impl ProcessSnapshot {
    /// Captures process topology via `ps -A`.
    /// Returns `None` if `ps` fails.
    pub fn capture() -> Option<Self> {
        let text = capture_process_table()?;
        Some(Self::from_ps_output(&text))
    }

    /// Captures command arguments for checks that distinguish subcommands sharing one executable.
    fn capture_with_command_lines_and_groups() -> Option<Self> {
        let processes = capture_process_groups_table()?;
        let args_output = process_args_command().output().ok()?;
        if !args_output.status.success() {
            return None;
        }
        let args_text = String::from_utf8_lossy(&args_output.stdout);
        Some(Self::from_ps_outputs_with_groups(&processes, &args_text))
    }

    fn from_ps_output(text: &str) -> Self {
        Self::from_process_table(text, false)
    }

    fn from_ps_outputs_with_groups(processes: &str, args: &str) -> Self {
        let mut snapshot = Self::from_process_table(processes, true);
        snapshot.command_lines = parse_ps_args_output(args);
        snapshot
    }

    fn from_process_table(text: &str, includes_groups: bool) -> Self {
        let mut children: HashMap<u32, Vec<(u32, String)>> = HashMap::new();
        let mut parents: HashMap<u32, u32> = HashMap::new();
        let mut comms: HashMap<u32, String> = HashMap::new();
        let mut command_lines: HashMap<u32, String> = HashMap::new();
        let mut pgids: HashMap<u32, libc::pid_t> = HashMap::new();
        for line in text.lines() {
            let mut columns = line.split_whitespace();
            let Some(pid) = columns.next().and_then(|value| value.parse::<u32>().ok()) else {
                continue;
            };
            let Some(ppid) = columns.next().and_then(|value| value.parse::<u32>().ok()) else {
                continue;
            };
            let pgid = if includes_groups {
                let Some(pgid) = columns
                    .next()
                    .and_then(|value| value.parse::<libc::pid_t>().ok())
                else {
                    continue;
                };
                Some(pgid)
            } else {
                None
            };
            let comm = columns.collect::<Vec<_>>().join(" ");
            if comm.is_empty() {
                continue;
            }
            children.entry(ppid).or_default().push((pid, comm.clone()));
            parents.insert(pid, ppid);
            comms.insert(pid, comm.clone());
            command_lines.insert(pid, comm);
            if let Some(pgid) = pgid {
                pgids.insert(pid, pgid);
            }
        }
        Self {
            children,
            parents,
            comms,
            command_lines,
            pgids,
        }
    }

    /// Returns `pid` and every ancestor up to (but not including) the point
    /// where the parent is unknown or a cycle is detected.
    ///
    /// Used to protect the calling process and the shell/tmux chain that
    /// invoked it from being killed when cleaning up processes still rooted
    /// in a worktree being deleted (e.g. `a agent close` run from inside it).
    pub fn ancestors(&self, pid: u32) -> HashSet<u32> {
        let mut result = HashSet::new();
        result.insert(pid);
        let mut current = pid;
        while let Some(&parent) = self.parents.get(&current) {
            if !result.insert(parent) {
                break;
            }
            current = parent;
        }
        result
    }

    /// Returns caller-pane descendants that should not block worktree removal.
    ///
    /// The caller-to-pane ancestry needs known cwd values outside the worktree.
    /// Known in-worktree cwd values above the pane still block removal.
    pub(super) fn caller_descendants_outside_path(
        &self,
        self_pid: u32,
        caller_pane_pid: Option<u32>,
        cwds: &HashMap<u32, PathBuf>,
        path: &Path,
    ) -> HashSet<u32> {
        let ancestors = self.ancestors(self_pid);
        if ancestors
            .iter()
            .any(|pid| *pid != 0 && cwds.get(pid).is_some_and(|cwd| cwd.starts_with(path)))
        {
            return HashSet::new();
        }

        let Some(caller_pane_pid) = caller_pane_pid.filter(|pane_pid| ancestors.contains(pane_pid))
        else {
            return HashSet::new();
        };

        let mut current = self_pid;
        loop {
            if cwds.get(&current).is_none_or(|cwd| cwd.starts_with(path)) {
                return HashSet::new();
            }
            if current == caller_pane_pid {
                break;
            }
            let Some(parent_pid) = self.parent_pid(current) else {
                return HashSet::new();
            };
            current = parent_pid;
        }

        let mut descendants = self.self_and_descendants(caller_pane_pid);
        descendants.retain(|pid| !ancestors.contains(pid));
        descendants
    }

    /// Returns the basename of the comm for `pid`, if known.
    pub fn comm_basename(&self, pid: u32) -> Option<&str> {
        self.comms
            .get(&pid)
            .map(|c| c.rsplit('/').next().unwrap_or(c.as_str()))
    }

    pub(super) fn parent_pid(&self, pid: u32) -> Option<u32> {
        self.parents.get(&pid).copied()
    }

    pub(super) fn self_and_descendants(&self, pid: u32) -> HashSet<u32> {
        let mut result = HashSet::from([pid]);
        let mut stack = vec![pid];
        while let Some(current) = stack.pop() {
            for &(child, _) in self.children.get(&current).into_iter().flatten() {
                if result.insert(child) {
                    stack.push(child);
                }
            }
        }
        result
    }

    pub(super) fn command_line(&self, pid: u32) -> Option<&str> {
        self.command_lines.get(&pid).map(String::as_str)
    }

    fn codex_app_server_ancestors(&self, pid: u32) -> impl Iterator<Item = u32> + '_ {
        self.ancestors(pid)
            .into_iter()
            .filter(move |&ancestor| ancestor != pid)
            .filter(|ancestor| {
                let Some(command_line) = self.command_lines.get(ancestor) else {
                    return false;
                };
                let mut args = command_line.split_whitespace();
                args.next()
                    .and_then(|executable| Path::new(executable).file_name())
                    .is_some_and(|name| name == "codex")
                    && args.next() == Some("app-server")
                    && args.any(|argument| argument == "--managed-daemon")
            })
    }

    fn pgid(&self, pid: u32) -> Option<libc::pid_t> {
        self.pgids.get(&pid).copied()
    }

    /// Resolves the pid of the first process in the subtree rooted at
    /// `start_pid` (inclusive) whose comm basename matches `target`.
    ///
    /// Unlike [`find_descendant_by_command`], this also considers `start_pid`
    /// itself, which matters when the pane command is `claude` directly (no
    /// shell wrapper).
    pub fn find_self_or_descendant_by_command(
        &self,
        start_pid: u32,
        target: &str,
        max_nodes: usize,
    ) -> Option<u32> {
        if self.comm_basename(start_pid) == Some(target) {
            return Some(start_pid);
        }
        self.find_descendant_by_command(start_pid, target, max_nodes)
    }

    /// BFS from `start_pid` (exclusive) looking for the first descendant whose
    /// comm basename matches `target`. Visits at most `max_nodes` processes.
    pub fn find_descendant_by_command(
        &self,
        start_pid: u32,
        target: &str,
        max_nodes: usize,
    ) -> Option<u32> {
        let mut queue: VecDeque<u32> = VecDeque::new();
        queue.push_back(start_pid);
        let mut visited = 0usize;

        while let Some(current) = queue.pop_front() {
            visited += 1;
            if visited > max_nodes {
                return None;
            }
            if let Some(kids) = self.children.get(&current) {
                for (child_pid, comm) in kids {
                    let basename = comm.rsplit('/').next().unwrap_or(comm);
                    if basename == target {
                        return Some(*child_pid);
                    }
                    queue.push_back(*child_pid);
                }
            }
        }
        None
    }
}

fn process_args_command() -> Command {
    let mut command = command::new("ps");
    command.args(["-A", "-ww", "-o", "pid=,args="]);
    command
}

fn capture_process_table() -> Option<String> {
    let output = command::new("ps")
        .args(["-A", "-o", "pid=,ppid=,comm="])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn capture_process_groups_table() -> Option<String> {
    let output = command::new("ps")
        .args(["-A", "-o", "pid=,ppid=,pgid=,comm="])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn parse_ps_args_output(text: &str) -> HashMap<u32, String> {
    text.lines()
        .filter_map(|line| {
            let mut columns = line.split_whitespace();
            let pid = columns.next()?.parse::<u32>().ok()?;
            let args = columns.collect::<Vec<_>>().join(" ");
            (!args.is_empty()).then_some((pid, args))
        })
        .collect()
}

/// Captures the current working directory of every process visible to the
/// caller via `lsof -a -d cwd -Fpn`, keyed by pid.
///
/// Returns `None` if `lsof` is unavailable or fails.
pub(super) fn list_process_cwds() -> Option<HashMap<u32, PathBuf>> {
    let output = ExternalTool::Lsof
        .command()
        .args(["-a", "-d", "cwd", "-Fpn"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(parse_lsof_cwd_output(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

/// Parses `lsof -Fpn` output, where each line is a single field prefixed
/// with its identifier letter (`p` for pid, `n` for file name; other
/// identifiers such as `f` for file descriptor are ignored).
fn parse_lsof_cwd_output(text: &str) -> HashMap<u32, PathBuf> {
    let mut result = HashMap::new();
    let mut current_pid: Option<u32> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix('p') {
            current_pid = rest.parse().ok();
        } else if let Some(rest) = line.strip_prefix('n')
            && let Some(pid) = current_pid
        {
            result.insert(pid, PathBuf::from(rest));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use indoc::indoc;
    use rstest::rstest;

    use super::*;

    #[test]
    fn process_args_command_requests_unlimited_width() {
        let args = process_args_command()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(args, ["-A", "-ww", "-o", "pid=,args="]);
    }

    #[rstest]
    #[case::pane_pid_is_claude_basename(
        indoc! {"
            100 1 /sbin/launchd
            200 100 claude
        "},
        200,
        Some(200),
    )]
    #[case::pane_pid_is_claude_fullpath(
        indoc! {"
            100 1 /sbin/launchd
            200 100 /usr/local/bin/claude
        "},
        200,
        Some(200),
    )]
    #[case::shell_child_is_claude(
        indoc! {"
            100 1 /sbin/launchd
            200 100 /bin/zsh
            300 200 /Users/example/.local/bin/claude
        "},
        200,
        Some(300),
    )]
    #[case::claude_nowhere(
        indoc! {"
            100 1 /sbin/launchd
            200 100 /bin/zsh
            300 200 vim
        "},
        200,
        None,
    )]
    #[case::grandchild_is_claude(
        indoc! {"
            100 1 /sbin/launchd
            200 100 /bin/zsh
            300 200 node
            400 300 claude
        "},
        200,
        Some(400),
    )]
    fn find_self_or_descendant_by_command_cases(
        #[case] ps_output: &str,
        #[case] start_pid: u32,
        #[case] expected: Option<u32>,
    ) {
        let snapshot = ProcessSnapshot::from_ps_output(ps_output);
        let got = snapshot.find_self_or_descendant_by_command(start_pid, "claude", 64);
        assert_eq!(got, expected);
    }

    #[rstest]
    #[case::known_basename("/usr/local/bin/claude", 200, Some("claude"))]
    #[case::bare_basename("claude", 200, Some("claude"))]
    #[case::unknown_pid("claude", 999, None)]
    fn comm_basename_cases(
        #[case] comm: &str,
        #[case] query_pid: u32,
        #[case] expected: Option<&str>,
    ) {
        let ps_output = format!("200 100 {comm}\n");
        let snapshot = ProcessSnapshot::from_ps_output(&ps_output);
        assert_eq!(snapshot.comm_basename(query_pid), expected);
    }

    #[rstest]
    #[case::walks_up_to_root(
        indoc! {"
            1 0 launchd
            100 1 tmux
            200 100 zsh
            300 200 a
        "},
        300,
        &[0, 1, 100, 200, 300],
    )]
    #[case::stops_at_unknown_parent(
        indoc! {"
            200 100 zsh
            300 200 a
        "},
        300,
        &[100, 200, 300],
    )]
    #[case::single_process(
        indoc! {"
            1 0 launchd
        "},
        1,
        &[0, 1],
    )]
    fn ancestors_cases(#[case] ps_output: &str, #[case] pid: u32, #[case] expected: &[u32]) {
        let snapshot = ProcessSnapshot::from_ps_output(ps_output);
        let mut got: Vec<u32> = snapshot.ancestors(pid).into_iter().collect();
        got.sort_unstable();
        let mut expected = expected.to_vec();
        expected.sort_unstable();
        assert_eq!(got, expected);
    }

    #[rstest]
    #[case::single_process(
        indoc! {"
            p100
            fcwd
            n/repo/.worktrees/feature
        "},
        &[(100, "/repo/.worktrees/feature")],
    )]
    #[case::multiple_processes(
        indoc! {"
            p100
            fcwd
            n/repo/.worktrees/feature
            p200
            fcwd
            n/
        "},
        &[(100, "/repo/.worktrees/feature"), (200, "/")],
    )]
    #[case::empty_output("", &[])]
    fn parse_lsof_cwd_output_cases(#[case] text: &str, #[case] expected: &[(u32, &str)]) {
        let got = parse_lsof_cwd_output(text);
        let expected: HashMap<u32, PathBuf> = expected
            .iter()
            .map(|&(pid, path)| (pid, PathBuf::from(path)))
            .collect();
        assert_eq!(got, expected);
    }
}
