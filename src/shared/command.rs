use std::path::{Path, PathBuf};

/// Check if a command is available in PATH.
pub fn is_command_available(cmd: &str) -> bool {
    find_command_path(cmd).is_some()
}

/// Find the full path of a command in PATH.
/// Returns the first matching executable path, or None if not found.
pub fn find_command_path(cmd: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;

    std::env::split_paths(&path_var).find_map(|dir| {
        let path = dir.join(cmd);
        if is_executable(&path) {
            Some(path)
        } else {
            None
        }
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

    #[test]
    fn test_find_command_path_returns_path_for_existing_command() {
        let path = find_command_path("sh");
        assert!(path.is_some());
        assert!(path.unwrap().to_string_lossy().contains("sh"));
    }

    #[test]
    fn test_find_command_path_returns_none_for_nonexistent_command() {
        let path = find_command_path("definitely-not-a-real-command-12345");
        assert!(path.is_none());
    }
}
