use std::time::Duration;
use std::{collections::BTreeSet, process::Command};

use anyhow::{Context, Result, bail};

use crate::infra::external_tool::ExternalTool;
use crate::infra::process;

pub(super) fn crit_pid_for_port(port: u16) -> Result<Option<u32>> {
    let status = crit_status()?;
    Ok(pid_for_port(&status, port))
}

pub(in crate::commands::agent::crit) fn crit_listener_pid_for_port(
    port: u16,
) -> Result<Option<u32>> {
    let output = process::run_with_timeout(crit_listener_command(port), Duration::from_secs(5))
        .context("failed to inspect crit daemon listener")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    crit_listener_pid_from_lsof_result(
        output.status.success(),
        output.status.code(),
        &stdout,
        &stderr,
    )
}

fn crit_listener_command(port: u16) -> Command {
    let mut command = ExternalTool::Lsof.command();
    command.args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-Fpc"]);
    command
}

fn crit_listener_pid_from_lsof_result(
    status_success: bool,
    exit_code: Option<i32>,
    stdout: &str,
    stderr: &str,
) -> Result<Option<u32>> {
    if !status_success {
        if exit_code == Some(1) && stdout.is_empty() && stderr.is_empty() {
            return Ok(None);
        }
        bail!("lsof failed to inspect crit daemon listener: {stderr}");
    }

    let mut current_pid = None;
    let mut current_command = None;
    let mut crit_pids = BTreeSet::new();
    for field in stdout.lines() {
        if let Some(pid) = field.strip_prefix('p') {
            record_crit_listener(&mut crit_pids, current_pid, current_command)?;
            current_pid = Some(
                pid.parse::<u32>()
                    .context("lsof returned an invalid process ID")?,
            );
            current_command = None;
        } else if let Some(command) = field.strip_prefix('c') {
            anyhow::ensure!(
                current_pid.is_some(),
                "lsof returned a command without a process"
            );
            current_command = Some(command);
        }
    }
    record_crit_listener(&mut crit_pids, current_pid, current_command)?;
    anyhow::ensure!(
        crit_pids.len() <= 1,
        "multiple crit daemons are listening on the same port"
    );
    Ok(crit_pids.into_iter().next())
}

fn record_crit_listener(
    crit_pids: &mut BTreeSet<u32>,
    pid: Option<u32>,
    command: Option<&str>,
) -> Result<()> {
    if let Some(pid) = pid {
        anyhow::ensure!(command.is_some(), "lsof omitted a process command");
        if command == Some("crit") {
            crit_pids.insert(pid);
        }
    }
    Ok(())
}

pub(super) fn session_for_port(
    status: &serde_json::Value,
    port: u16,
) -> Option<&serde_json::Value> {
    status
        .get("sessions")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .find(|session| {
            session.get("port").and_then(serde_json::Value::as_u64) == Some(port.into())
        })
}

pub(in crate::commands::agent::crit) fn pid_for_port(
    status: &serde_json::Value,
    port: u16,
) -> Option<u32> {
    session_for_port(status, port)
        .and_then(|session| session.get("pid").and_then(serde_json::Value::as_u64))
        .and_then(|pid| u32::try_from(pid).ok())
}

pub(in crate::commands::agent::crit) fn crit_status() -> Result<serde_json::Value> {
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

#[cfg(test)]
mod tests {
    use super::crit_listener_pid_from_lsof_result;

    #[test]
    fn returns_none_when_lsof_finds_no_listener_for_the_port() {
        assert_eq!(
            crit_listener_pid_from_lsof_result(false, Some(1), "", "")
                .expect("no matching lsof entry should be treated as an absent listener"),
            None,
        );
    }

    #[test]
    fn returns_pid_when_lsof_finds_a_crit_listener() {
        assert_eq!(
            crit_listener_pid_from_lsof_result(
                true,
                Some(0),
                indoc::indoc! {"
                    p1234
                    ccrit
                "},
                "",
            )
            .expect("valid crit listener output should parse"),
            Some(1234),
        );
    }
}
