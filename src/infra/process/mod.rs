//! Process-tree utilities (parent PID lookup, descendant search).
//!
//! All external-process interaction (currently `ps`) is isolated in this
//! module so that production code elsewhere can call pure functions and tests
//! can stub at the module boundary.

use std::collections::{HashMap, VecDeque};
use std::process::Command;

/// Looks up the parent PID of `pid` using `ps -o ppid= -p <pid>`.
/// Returns `None` if the process is gone or `ps` fails.
pub fn get_parent_pid(pid: u32) -> Option<u32> {
    let output = Command::new("ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u32>()
        .ok()
}

/// A single snapshot of the process table, taken once and queried many times.
///
/// Stores a parent → children mapping so callers can search for descendants
/// without forking `ps` for every lookup.
pub struct ProcessSnapshot {
    children: HashMap<u32, Vec<(u32, String)>>,
    comms: HashMap<u32, String>,
}

impl ProcessSnapshot {
    /// Captures the current process table via `ps -A`.
    /// Returns `None` if `ps` fails.
    pub fn capture() -> Option<Self> {
        let output = Command::new("ps")
            .args(["-A", "-o", "pid=,ppid=,comm="])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        Some(Self::from_ps_output(&text))
    }

    fn from_ps_output(text: &str) -> Self {
        let mut children: HashMap<u32, Vec<(u32, String)>> = HashMap::new();
        let mut comms: HashMap<u32, String> = HashMap::new();
        for line in text.lines() {
            let mut it = line.split_whitespace();
            let Some(pid) = it.next().and_then(|s| s.parse::<u32>().ok()) else {
                continue;
            };
            let Some(ppid) = it.next().and_then(|s| s.parse::<u32>().ok()) else {
                continue;
            };
            let comm: String = it.collect::<Vec<_>>().join(" ");
            if comm.is_empty() {
                continue;
            }
            children.entry(ppid).or_default().push((pid, comm.clone()));
            comms.insert(pid, comm);
        }
        Self { children, comms }
    }

    /// Returns the basename of the comm for `pid`, if known.
    pub fn comm_basename(&self, pid: u32) -> Option<&str> {
        self.comms
            .get(&pid)
            .map(|c| c.rsplit('/').next().unwrap_or(c.as_str()))
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

#[cfg(test)]
mod tests {
    use indoc::indoc;
    use rstest::rstest;

    use super::*;

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
            300 200 /Users/fohte/.local/bin/claude
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
}
