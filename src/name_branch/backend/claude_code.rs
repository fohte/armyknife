use std::io::Write;
use std::process::Command;

use super::Backend;
use crate::name_branch::error::{Error, Result};

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

        extract_first_line(&output.stdout)
    }
}

fn extract_first_line(stdout: &[u8]) -> Result<String> {
    let result = String::from_utf8_lossy(stdout)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();

    if result.is_empty() {
        return Err(Error::GenerationFailed(
            "claude returned empty output".to_string(),
        ));
    }

    Ok(result)
}
