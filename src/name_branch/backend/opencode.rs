use std::process::Command;

use super::Backend;
use crate::name_branch::error::{Error, Result};

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
            "opencode returned empty output".to_string(),
        ));
    }

    Ok(result)
}
