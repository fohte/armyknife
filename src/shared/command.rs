use std::path::Path;

/// Check if a command is available in PATH.
pub fn is_command_available(cmd: &str) -> bool {
    let path_var = match std::env::var_os("PATH") {
        Some(p) => p,
        None => return false,
    };

    std::env::split_paths(&path_var).any(|dir| {
        let path = dir.join(cmd);
        is_executable(&path)
    })
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_file()
        && path
            .metadata()
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_common_commands_available() {
        // These commands should be available on most systems
        assert!(is_command_available("sh"));
    }

    #[test]
    fn test_nonexistent_command_not_available() {
        assert!(!is_command_available("definitely-not-a-real-command-12345"));
    }
}
