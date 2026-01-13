mod claude_code;
mod opencode;

pub use claude_code::ClaudeCode;
pub use opencode::OpenCode;

use std::process::Output;

use super::error::{Error, Result};

/// Backend trait for generating text from a prompt.
///
/// This abstraction allows swapping LLM backends (Claude Code, Codex, Gemini CLI, etc.)
pub trait Backend {
    fn generate(&self, prompt: &str) -> Result<String>;
}

/// Detect and return the best available backend.
/// Priority: Claude Code > OpenCode
pub fn detect_backend() -> Box<dyn Backend> {
    if is_command_available("claude") {
        Box::new(ClaudeCode)
    } else {
        Box::new(OpenCode)
    }
}

/// Check if a command is available in PATH.
fn is_command_available(cmd: &str) -> bool {
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
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_file()
        && path
            .metadata()
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &std::path::Path) -> bool {
    path.is_file()
}

/// Check command output status and return error if failed.
pub(super) fn check_command_status(output: &Output, command_name: &str) -> Result<()> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::GenerationFailed(format!(
            "{command_name} exited with status {}: {}",
            output.status, stderr
        )));
    }
    Ok(())
}

/// Extract the first non-empty line from stdout.
pub(super) fn extract_first_line(stdout: &[u8], command_name: &str) -> Result<String> {
    let result = String::from_utf8_lossy(stdout)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();

    if result.is_empty() {
        return Err(Error::GenerationFailed(format!(
            "{command_name} returned empty output"
        )));
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock backend for testing
    pub struct MockBackend {
        pub response: String,
    }

    impl Backend for MockBackend {
        fn generate(&self, _prompt: &str) -> Result<String> {
            Ok(self.response.clone())
        }
    }

    #[test]
    fn test_mock_backend() {
        let backend = MockBackend {
            response: "fix-login-bug".to_string(),
        };
        let result = backend.generate("test prompt").unwrap();
        assert_eq!(result, "fix-login-bug");
    }

    #[test]
    fn test_is_command_available_finds_common_commands() {
        // sh should exist on all Unix-like systems
        assert!(is_command_available("sh"));
    }

    #[test]
    fn test_is_command_available_returns_false_for_nonexistent() {
        assert!(!is_command_available("nonexistent-command-12345"));
    }
}
