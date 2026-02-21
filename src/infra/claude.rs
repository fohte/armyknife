use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Result, bail};

use crate::shared::env_var::EnvVars;

/// Runs `claude -p` with a system prompt and user prompt piped to stdin.
///
/// Returns the trimmed stdout output.
/// Tools are disabled (`--tools ""`) to prevent the model from attempting
/// tool calls (e.g., Edit) that produce permission prompts in stdout.
///
/// `max_output_tokens` sets `CLAUDE_CODE_MAX_OUTPUT_TOKENS` on the child
/// process, causing Claude Code to reject responses that exceed the limit.
pub fn run_print_mode(
    model: &str,
    system_prompt: &str,
    user_prompt: &str,
    max_output_tokens: Option<u32>,
) -> Result<String> {
    let (skip_key, skip_val) = EnvVars::skip_hooks_pair();
    let mut cmd = Command::new("claude");
    cmd.args(["-p", "--model", model, "--tools", ""]);
    if !system_prompt.is_empty() {
        cmd.args(["--system-prompt", system_prompt]);
    }
    // Prevent hooks from firing in the child claude process,
    // which would cause infinite recursion (hook → claude -p → hook → ...).
    cmd.env(skip_key, skip_val);
    if let Some(tokens) = max_output_tokens {
        cmd.env("CLAUDE_CODE_MAX_OUTPUT_TOKENS", tokens.to_string());
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;

    if let Some(ref mut stdin) = child.stdin {
        let _ = stdin.write_all(user_prompt.as_bytes());
    }

    let output = child.wait_with_output()?;

    if !output.status.success() {
        let msg = String::from_utf8_lossy(&output.stdout).trim().to_string();
        bail!("claude exited with {}: {msg}", output.status);
    }

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if text.is_empty() {
        bail!("claude returned empty output");
    }

    Ok(text)
}

/// Spawns the current binary as a fully detached background process.
///
/// All stdio streams are redirected to null. Errors are silently ignored
/// because background tasks are best-effort.
pub fn spawn_self_detached(args: &[&str]) {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(_) => return,
    };

    let _ = Command::new(exe)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}
