//! Process-tree utilities (parent PID lookup, descendant search).
//!
//! All external-process interaction (currently `ps`) is isolated in this
//! module so that production code elsewhere can call pure functions and tests
//! can stub at the module boundary.

use std::collections::VecDeque;
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

/// Walks the process tree rooted at `start_pid` (exclusive) breadth-first,
/// returning the first descendant whose `ps -o comm=` output (basename)
/// matches `target`.
///
/// The walk visits at most `max_nodes` processes. Callers should set this
/// to a modest value (e.g., 64) because we only need to cross a shell or two
/// to reach the claude process inside a tmux pane -- larger limits invite
/// pathological trees (build systems with fan-out > 1000).
pub fn find_descendant_by_command(start_pid: u32, target: &str, max_nodes: usize) -> Option<u32> {
    // Take one ps snapshot and build a pid->children map so the walk doesn't
    // have to fork per visited node.
    let snapshot = match Command::new("ps")
        .args(["-A", "-o", "pid=,ppid=,comm="])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return None,
    };
    let text = String::from_utf8_lossy(&snapshot.stdout);

    // Parse once into (pid, ppid, comm) tuples.
    let rows: Vec<(u32, u32, String)> = text
        .lines()
        .filter_map(|line| {
            // Split into at most 3 parts so that comm (which may contain
            // spaces on macOS) is preserved intact: "<pid> <ppid> <comm>".
            let line = line.trim_start();
            let mut it = line.splitn(3, char::is_whitespace);
            let pid: u32 = it.next()?.parse().ok()?;
            let ppid_str = it.next()?.trim();
            let ppid: u32 = ppid_str.parse().ok()?;
            let comm = it.next()?.trim().to_string();
            Some((pid, ppid, comm))
        })
        .collect();

    let mut queue: VecDeque<u32> = VecDeque::new();
    queue.push_back(start_pid);
    let mut visited = 0usize;

    while let Some(current) = queue.pop_front() {
        visited += 1;
        if visited > max_nodes {
            return None;
        }

        // Enqueue children and check comm on each child.
        for (child_pid, _, comm) in rows.iter().filter(|(_, ppid, _)| *ppid == current) {
            let basename = comm.rsplit('/').next().unwrap_or(comm);
            if basename == target {
                return Some(*child_pid);
            }
            queue.push_back(*child_pid);
        }
    }
    None
}
