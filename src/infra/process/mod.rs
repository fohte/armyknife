//! Process-tree utilities (parent PID lookup, command name lookup).
//!
//! All external-process interaction (currently `ps`) is isolated in this
//! module so that production code elsewhere can call pure functions and tests
//! can stub at the module boundary.

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

/// Looks up the command name (comm, not full argv) of `pid`.
/// Returns `None` if the process is gone or `ps` fails.
pub fn get_command_name(pid: u32) -> Option<String> {
    let output = Command::new("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}

/// Walks up the process tree starting from `start_pid` (exclusive) looking for
/// the first ancestor whose `ps -o comm=` output matches `target`.
/// `target` matching is done on the basename only (last path segment) so that
/// `claude` matches both `claude` and `/usr/local/bin/claude`.
///
/// Returns the matching ancestor PID, or `None` if nothing matches within
/// `max_depth` hops (default 20 for hook contexts).
pub fn find_ancestor_by_command(start_pid: u32, target: &str, max_depth: usize) -> Option<u32> {
    let mut current = start_pid;
    for _ in 0..max_depth {
        let parent = get_parent_pid(current)?;
        if parent == 0 || parent == current {
            return None;
        }
        if let Some(name) = get_command_name(parent) {
            let basename = name.rsplit('/').next().unwrap_or(&name);
            if basename == target {
                return Some(parent);
            }
        }
        current = parent;
    }
    None
}
