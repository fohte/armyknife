mod claude_code;
mod opencode;

pub use claude_code::ClaudeCode;
pub use opencode::OpenCode;

use super::error::Result;

/// Backend trait for generating text from a prompt.
///
/// This abstraction allows swapping LLM backends (Claude Code, Codex, Gemini CLI, etc.)
pub trait Backend {
    fn generate(&self, prompt: &str) -> Result<String>;
}

/// Detect and return the best available backend.
/// Priority: OpenCode > Claude Code
pub fn detect_backend() -> Box<dyn Backend> {
    if is_command_available("opencode") {
        Box::new(OpenCode)
    } else {
        Box::new(ClaudeCode)
    }
}

/// Check if a command is available in PATH.
fn is_command_available(cmd: &str) -> bool {
    let path_var = match std::env::var_os("PATH") {
        Some(p) => p,
        None => return false,
    };

    std::env::split_paths(&path_var).any(|dir| dir.join(cmd).is_file())
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
