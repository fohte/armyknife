use std::process::Command;

/// Maximum number of parent processes to traverse when looking for Claude Code process.
const MAX_ANCESTOR_DEPTH: usize = 20;

/// Gets the Claude Code process PID from ancestor processes.
/// Traverses up to MAX_ANCESTOR_DEPTH levels of parent processes to find a Claude Code process.
/// Returns the first ancestor PID that is running Claude Code (claude or node process).
pub fn get_claude_pid_from_ancestors() -> Option<u32> {
    let mut pid = std::process::id();

    for _ in 0..MAX_ANCESTOR_DEPTH {
        if is_claude_process(pid) {
            return Some(pid);
        }

        // Get parent PID and continue traversing
        match get_parent_pid(pid) {
            Some(ppid) if ppid != pid && ppid != 0 => pid = ppid,
            _ => break,
        }
    }

    None
}

/// Checks if a process is a Claude Code process.
/// Claude Code runs as a node process with the claude CLI.
fn is_claude_process(pid: u32) -> bool {
    let output = Command::new("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .ok();

    match output {
        Some(output) if output.status.success() => {
            let comm = String::from_utf8_lossy(&output.stdout);
            let comm = comm.trim();
            // Claude Code runs as "node" or might be named "claude"
            comm == "node" || comm == "claude"
        }
        _ => false,
    }
}

/// Gets the parent process ID for a given process.
fn get_parent_pid(pid: u32) -> Option<u32> {
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

/// Checks if a process is still alive.
pub fn is_process_alive(pid: u32) -> bool {
    // Use ps to check if process exists (works on both macOS and Linux)
    Command::new("ps")
        .args(["-p", &pid.to_string()])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[test]
    fn test_is_process_alive_current_process() {
        // Current process should always be alive
        let pid = std::process::id();
        assert!(is_process_alive(pid));
    }

    #[test]
    fn test_is_process_alive_nonexistent_process() {
        // PID 0 is the kernel scheduler, should not be accessible
        // PID 99999999 is unlikely to exist
        assert!(!is_process_alive(99999999));
    }

    #[rstest]
    fn test_get_parent_pid_current_process() {
        let pid = std::process::id();
        let ppid = get_parent_pid(pid);
        // Current process should have a parent
        assert!(ppid.is_some());
        // Parent PID should be different from current PID
        assert_ne!(ppid.unwrap(), pid);
    }
}
