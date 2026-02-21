use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Result, bail};

/// Runs `claude -p` with a system prompt and user prompt piped to stdin.
///
/// Returns the trimmed stdout output.
/// Tools are disabled (`--tools ""`) to prevent the model from attempting
/// tool calls (e.g., Edit) that produce permission prompts in stdout.
pub fn run_print_mode(model: &str, system_prompt: &str, user_prompt: &str) -> Result<String> {
    let mut child = Command::new("claude")
        .args([
            "-p",
            "--model",
            model,
            "--system-prompt",
            system_prompt,
            "--tools",
            "",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;

    if let Some(ref mut stdin) = child.stdin {
        let _ = stdin.write_all(user_prompt.as_bytes());
    }

    let output = child.wait_with_output()?;
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
