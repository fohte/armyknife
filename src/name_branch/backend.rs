use std::io::Write;
use std::process::Command;

use super::error::{Error, Result};

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

fn is_command_available(cmd: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {cmd}"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// OpenCode backend using `opencode run -m opencode/glm-4.7-free`
pub struct OpenCode;

impl Backend for OpenCode {
    fn generate(&self, prompt: &str) -> Result<String> {
        let output = Command::new("opencode")
            .args(["run", "-m", "opencode/glm-4.7-free", prompt])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .map_err(|e| Error::GenerationFailed(format!("Failed to run opencode: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::GenerationFailed(format!(
                "opencode exited with status {}: {}",
                output.status, stderr
            )));
        }

        extract_first_line(&output.stdout, "opencode")
    }
}

/// Claude Code backend using `claude --model haiku --print`
pub struct ClaudeCode;

impl Backend for ClaudeCode {
    fn generate(&self, prompt: &str) -> Result<String> {
        let mut child = Command::new("claude")
            .args(["--model", "haiku", "--print"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| Error::GenerationFailed(format!("Failed to spawn claude: {e}")))?;

        let mut stdin = child.stdin.take().ok_or_else(|| {
            Error::GenerationFailed("Failed to get stdin handle for claude process".to_string())
        })?;
        stdin
            .write_all(prompt.as_bytes())
            .map_err(|e| Error::GenerationFailed(format!("Failed to write to stdin: {e}")))?;
        drop(stdin);

        let output = child
            .wait_with_output()
            .map_err(|e| Error::GenerationFailed(format!("Failed to wait for claude: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::GenerationFailed(format!(
                "claude exited with status {}: {}",
                output.status, stderr
            )));
        }

        extract_first_line(&output.stdout, "claude")
    }
}

fn extract_first_line(stdout: &[u8], cmd_name: &str) -> Result<String> {
    let result = String::from_utf8_lossy(stdout)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();

    if result.is_empty() {
        return Err(Error::GenerationFailed(format!(
            "{cmd_name} returned empty output"
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
}
