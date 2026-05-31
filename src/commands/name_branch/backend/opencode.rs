use anyhow::Context;

use super::{Backend, check_command_status, extract_first_line};
use crate::commands::name_branch::error::Result;
use crate::infra::external_tool::ExternalTool;

/// OpenCode backend using `opencode run -m opencode/glm-4.7-free`
pub struct OpenCode;

impl Backend for OpenCode {
    fn generate(&self, prompt: &str) -> Result<String> {
        let output = ExternalTool::Opencode
            .command()
            .args(["run", "-m", "opencode/glm-4.7-free", prompt])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .context("Failed to run opencode")?;

        check_command_status(&output, "opencode")?;
        extract_first_line(&output.stdout, "opencode")
    }
}
