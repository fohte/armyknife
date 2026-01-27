mod claude_code;
mod opencode;

pub use claude_code::ClaudeCode;
pub use opencode::OpenCode;

use std::process::Output;

use anyhow::bail;

use super::error::Result;
use crate::shared::command::is_command_available;

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

/// Check command output status and return error if failed.
pub(super) fn check_command_status(output: &Output, command_name: &str) -> Result<()> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "{command_name} exited with status {}: {}",
            output.status,
            stderr
        );
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
        bail!("{command_name} returned empty output");
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
