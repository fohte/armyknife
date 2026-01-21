use std::io::Write;
use std::process::Command;

use super::{Backend, check_command_status, extract_first_line};
use crate::commands::name_branch::error::{Error, Result};

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

        check_command_status(&output, "claude")?;
        extract_first_line(&output.stdout, "claude")
    }
}
