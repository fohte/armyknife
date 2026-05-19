//! Thin wrapper around `launchctl` for managing launchd services.

use std::process::Stdio;

use anyhow::{Result, bail};

use crate::shared::command;

/// Returns true if the given service target (e.g., `gui/501/com.example.foo`)
/// is currently bootstrapped in the launchd domain.
pub fn is_bootstrapped(target: &str) -> bool {
    command::new("launchctl")
        .args(["print", target])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Runs `launchctl` with the given arguments, returning an error if the
/// command exits non-zero.
pub fn run(args: &[&str]) -> Result<()> {
    let output = command::new("launchctl").args(args).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("launchctl {:?} failed: {}", args, stderr.trim());
    }
    Ok(())
}
