use std::time::Duration;

use crate::infra::external_tool::ExternalTool;
use crate::infra::process;
use anyhow::{Context, Result, bail};

pub(super) fn crit_pid_for_port(port: u16) -> Result<Option<u32>> {
    let status = crit_status()?;
    Ok(pid_for_port(&status, port))
}

fn pid_for_port(status: &serde_json::Value, port: u16) -> Option<u32> {
    status
        .get("sessions")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .find(|session| {
            session.get("port").and_then(serde_json::Value::as_u64) == Some(port.into())
        })
        .and_then(|session| session.get("pid").and_then(serde_json::Value::as_u64))
        .and_then(|pid| u32::try_from(pid).ok())
}

fn crit_status() -> Result<serde_json::Value> {
    let mut command = ExternalTool::Crit.command();
    command.args(["status", "--json"]);
    let output = process::run_with_timeout(command, Duration::from_secs(5))
        .context("failed to read crit daemon status")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("crit status --json failed: {stderr}");
    }

    serde_json::from_slice(&output.stdout).context("failed to parse crit status JSON")
}
