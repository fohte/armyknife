//! shpool session queries shared by crit panes and review editor panes.

use anyhow::{Context, Result, ensure};
use serde::Deserialize;

use super::external_tool::ExternalTool;
use super::process::is_process_alive;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct Session {
    name: String,
    shell_pid: u32,
}

#[derive(Deserialize)]
struct SessionList {
    sessions: Vec<Session>,
}

impl Session {
    /// shpool keeps listing a session whose process exited while no client was
    /// attached, so the listing alone cannot tell whether the command still runs.
    fn is_alive(&self) -> bool {
        is_process_alive(self.shell_pid)
    }
}

pub(crate) fn session_is_alive(name: &str) -> Result<bool> {
    Ok(list_sessions()?
        .iter()
        .any(|session| session.name == name && session.is_alive()))
}

fn list_sessions() -> Result<Vec<Session>> {
    let output = ExternalTool::Shpool
        .command()
        .args(["list", "--json"])
        .output()
        .context("failed to run shpool list")?;
    ensure!(
        output.status.success(),
        "shpool list exited with {}",
        output.status
    );
    parse_sessions(&output.stdout)
}

fn parse_sessions(json: &[u8]) -> Result<Vec<Session>> {
    let list: SessionList =
        serde_json::from_slice(json).context("failed to parse shpool list output")?;
    Ok(list.sessions)
}

#[cfg(test)]
mod tests {
    use indoc::indoc;

    use super::*;

    #[test]
    fn parse_sessions_keeps_only_the_fields_armyknife_uses() {
        let json = indoc! {r#"
            {
              "sessions": [
                {
                  "name": "sample-session",
                  "started_at_unix_ms": 1700000000000,
                  "last_connected_at_unix_ms": 1700000000100,
                  "last_disconnected_at_unix_ms": null,
                  "status": "Attached",
                  "shell_pid": 4242,
                  "attachments": []
                }
              ]
            }
        "#};

        assert_eq!(
            parse_sessions(json.as_bytes()).map_err(|error| error.to_string()),
            Ok(vec![Session {
                name: "sample-session".to_string(),
                shell_pid: 4242,
            }]),
        );
    }

    #[test]
    fn parse_sessions_rejects_output_without_sessions() {
        assert_eq!(
            parse_sessions(b"{}").map_err(|error| error.to_string()),
            Err("failed to parse shpool list output".to_string()),
        );
    }

    #[test]
    fn is_alive_follows_the_shell_process() {
        let session = |shell_pid| Session {
            name: "sample-session".to_string(),
            shell_pid,
        };

        assert_eq!(
            (
                session(std::process::id()).is_alive(),
                session(u32::MAX).is_alive()
            ),
            (true, false),
        );
    }
}
