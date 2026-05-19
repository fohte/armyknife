use std::io::Write;

use anyhow::Context;

use super::{Backend, check_command_status, extract_first_line};
use crate::commands::name_branch::error::Result;
use crate::shared::command;

/// Claude Code backend using `claude --model haiku --print`
pub struct ClaudeCode;

impl Backend for ClaudeCode {
    fn generate(&self, prompt: &str) -> Result<String> {
        let mut child = command::new("claude")
            .args(["--model", "haiku", "--print"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .context("Failed to spawn claude")?;

        let mut stdin = child
            .stdin
            .take()
            .context("Failed to get stdin handle for claude process")?;
        stdin
            .write_all(prompt.as_bytes())
            .context("Failed to write to stdin")?;
        drop(stdin);

        let output = child
            .wait_with_output()
            .context("Failed to wait for claude")?;

        check_command_status(&output, "claude")?;
        extract_first_line(&output.stdout, "claude")
    }
}
