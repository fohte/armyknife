use std::path::Path;
use std::process::Command;

/// Maximum number of parent processes to traverse when looking for TTY.
const MAX_ANCESTOR_DEPTH: usize = 10;

/// Gets the TTY device from ancestor processes.
/// Traverses up to MAX_ANCESTOR_DEPTH levels of parent processes to find a valid TTY.
pub fn get_tty_from_ancestors() -> Option<String> {
    let mut pid = std::process::id();

    for _ in 0..MAX_ANCESTOR_DEPTH {
        if let Some(tty) = get_process_tty(pid)
            && is_valid_tty(&tty)
        {
            return Some(tty);
        }

        // Get parent PID and continue traversing
        match get_parent_pid(pid) {
            Some(ppid) if ppid != pid && ppid != 0 => pid = ppid,
            _ => break,
        }
    }

    None
}

/// Gets the TTY for a specific process ID.
fn get_process_tty(pid: u32) -> Option<String> {
    let output = Command::new("ps")
        .args(["-o", "tty=", "-p", &pid.to_string()])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let tty = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if tty.is_empty() || tty == "?" || tty == "??" {
        return None;
    }

    // Convert short form (ttys001) to full path (/dev/ttys001)
    if !tty.starts_with('/') {
        Some(format!("/dev/{tty}"))
    } else {
        Some(tty)
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

/// Checks if a TTY string represents a valid terminal device.
pub fn is_valid_tty(tty: &str) -> bool {
    // Must be an absolute path
    if !tty.starts_with('/') {
        return false;
    }

    // Common TTY patterns on macOS and Linux
    let valid_patterns = [
        "/dev/tty",     // Generic TTY
        "/dev/pts/",    // Linux pseudo-terminals
        "/dev/ttys",    // macOS pseudo-terminals
        "/dev/pty",     // BSD-style pseudo-terminals
        "/dev/console", // Console
    ];

    valid_patterns
        .iter()
        .any(|pattern| tty.starts_with(pattern))
}

/// Checks if a TTY device still exists and is accessible.
pub fn is_tty_alive(tty: &str) -> bool {
    Path::new(tty).exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::macos_ttys("/dev/ttys001", true)]
    #[case::macos_ttys_high("/dev/ttys999", true)]
    #[case::linux_pts("/dev/pts/0", true)]
    #[case::linux_pts_high("/dev/pts/123", true)]
    #[case::generic_tty("/dev/tty", true)]
    #[case::generic_tty_num("/dev/tty0", true)]
    #[case::console("/dev/console", true)]
    #[case::pty("/dev/pty0", true)]
    #[case::relative_path("ttys001", false)]
    #[case::not_tty("/dev/null", false)]
    #[case::home_path("/home/user", false)]
    #[case::empty("", false)]
    #[case::question_mark("?", false)]
    fn test_is_valid_tty(#[case] input: &str, #[case] expected: bool) {
        assert_eq!(is_valid_tty(input), expected);
    }

    #[test]
    fn test_is_tty_alive_with_existing_path() {
        // /dev/null always exists
        assert!(is_tty_alive("/dev/null"));
    }

    #[test]
    fn test_is_tty_alive_with_nonexistent_path() {
        assert!(!is_tty_alive("/dev/nonexistent_tty_12345"));
    }
}
