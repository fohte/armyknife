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
            children.entry(ppid).or_default().push((pid, comm));
        }
        Self { children }
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
