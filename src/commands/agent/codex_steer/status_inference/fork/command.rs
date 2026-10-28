use std::path::Path;
use std::process::Command;

use anyhow::{Result, ensure};
use serde_json::to_string;

use super::super::super::history::CodexThreadMetadata;
use super::home::PrivateCodexHome;
use super::path_to_string;

pub(super) struct ForkCommandOptions<'a> {
    pub(super) cwd: &'a Path,
    pub(super) thread: &'a CodexThreadMetadata,
    pub(super) prompt: &'a str,
    pub(super) result_path: &'a Path,
    pub(super) settings_path: &'a Path,
    pub(super) base_url: &'a str,
    pub(super) inference_dir: &'a Path,
    pub(super) private_home: &'a PrivateCodexHome,
}

pub(super) fn codex_fork_command(
    options: ForkCommandOptions<'_>,
    codex_path: &Path,
    mut command: Command,
) -> Result<Command> {
    let ForkCommandOptions {
        cwd,
        thread,
        prompt,
        result_path,
        settings_path,
        base_url,
        inference_dir,
        private_home,
    } = options;
    command
        .arg("--settings")
        .arg(settings_path)
        .arg(codex_path)
        .args(codex_fork_args(
            &thread.session_id,
            &thread.model,
            prompt,
            result_path,
            base_url,
            &private_home.sqlite_home,
            &private_home.log_dir,
        )?)
        .current_dir(cwd);
    configure_isolated_environment(&mut command, inference_dir, private_home);
    Ok(command)
}

fn configure_isolated_environment(
    command: &mut Command,
    inference_dir: &Path,
    private_home: &PrivateCodexHome,
) {
    command
        .env("ARMYKNIFE_SKIP_HOOKS", "1")
        .env("CLAUDE_CODE_TMPDIR", inference_dir)
        .env("CLAUDE_TMPDIR", inference_dir)
        .env("CODEX_HOME", &private_home.codex_home)
        .env("CODEX_SQLITE_HOME", &private_home.sqlite_home)
        .env("TMPDIR", &private_home.temp_dir)
        .env("TMP", &private_home.temp_dir)
        .env("TEMP", &private_home.temp_dir)
        .env_remove("ARMYKNIFE_SESSION_ID")
        .env_remove("ARMYKNIFE_ANCESTOR_SESSION_IDS")
        .env_remove("TQ_SESSION_ID")
        .env_remove("TQ_PARENT_SESSION_ID")
        .env_remove("CLAUDE_CODE_SESSION_ID")
        .env_remove("CODEX_SESSION_ID")
        .env_remove("CODEX_THREAD_ID")
        .env_remove("OPENAI_API_KEY")
        .env_remove("OPENAI_BASE_URL")
        .env_remove("CODEX_API_KEY")
        .env_remove("CODEX_ACCESS_TOKEN");
}

fn codex_fork_args(
    session_id: &str,
    model: &str,
    prompt: &str,
    result_path: &Path,
    base_url: &str,
    sqlite_home: &Path,
    log_dir: &Path,
) -> Result<Vec<String>> {
    ensure!(!session_id.is_empty(), "Codex parent session ID is empty");
    ensure!(!model.is_empty(), "Codex parent model is empty");
    Ok(vec![
        "exec".to_string(),
        "fork".to_string(),
        "--ephemeral".to_string(),
        "--model".to_string(),
        model.to_string(),
        "--config".to_string(),
        "model_provider=\"openai\"".to_string(),
        "--config".to_string(),
        format!(
            "sqlite_home={}",
            serde_json::to_string(&path_to_string(sqlite_home)?)?
        ),
        "--config".to_string(),
        format!(
            "log_dir={}",
            serde_json::to_string(&path_to_string(log_dir)?)?
        ),
        "--config".to_string(),
        "cli_auth_credentials_store=\"file\"".to_string(),
        "--config".to_string(),
        "mcp_oauth_credentials_store=\"file\"".to_string(),
        "--config".to_string(),
        format!("openai_base_url={}", to_string(base_url)?),
        "--config".to_string(),
        "features.hooks=false".to_string(),
        "--config".to_string(),
        "features.enable_request_compression=false".to_string(),
        "--output-last-message".to_string(),
        path_to_string(result_path)?,
        "--".to_string(),
        session_id.to_string(),
        prompt.to_string(),
    ])
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::process::Command;

    use super::{ForkCommandOptions, codex_fork_args, codex_fork_command};
    use crate::commands::agent::codex_steer::status_inference::fork::home::PrivateCodexHome;

    #[test]
    fn codex_fork_uses_parent_model_ephemeral_history_and_relay_overrides() {
        let actual = codex_fork_args(
            "fixture-parent-session",
            "fixture-model",
            "classify this stopped task",
            Path::new("/private/tmp/status-worker-fixture/result.json"),
            "http://localhost:43817/fixture-token/backend-api/codex",
            Path::new("/private/tmp/status-worker-fixture/codex-home/state"),
            Path::new("/private/tmp/status-worker-fixture/codex-home/logs"),
        );
        let actual = actual.expect("fixture fork arguments should be valid");
        assert_eq!(
            actual,
            vec![
                "exec",
                "fork",
                "--ephemeral",
                "--model",
                "fixture-model",
                "--config",
                "model_provider=\"openai\"",
                "--config",
                "sqlite_home=\"/private/tmp/status-worker-fixture/codex-home/state\"",
                "--config",
                "log_dir=\"/private/tmp/status-worker-fixture/codex-home/logs\"",
                "--config",
                "cli_auth_credentials_store=\"file\"",
                "--config",
                "mcp_oauth_credentials_store=\"file\"",
                "--config",
                "openai_base_url=\"http://localhost:43817/fixture-token/backend-api/codex\"",
                "--config",
                "features.hooks=false",
                "--config",
                "features.enable_request_compression=false",
                "--output-last-message",
                "/private/tmp/status-worker-fixture/result.json",
                "--",
                "fixture-parent-session",
                "classify this stopped task",
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>(),
        );
    }

    #[test]
    fn assembled_command_uses_private_paths_and_clears_inherited_identity() -> anyhow::Result<()> {
        let private_home = PrivateCodexHome {
            codex_home: Path::new("/private/tmp/status-worker-fixture/codex-home").to_path_buf(),
            sqlite_home: Path::new("/private/tmp/status-worker-fixture/codex-home/state")
                .to_path_buf(),
            log_dir: Path::new("/private/tmp/status-worker-fixture/codex-home/logs").to_path_buf(),
            temp_dir: Path::new("/private/tmp/status-worker-fixture/codex-home/tmp").to_path_buf(),
        };
        let command = codex_fork_command(
            ForkCommandOptions {
                cwd: Path::new("/work/fixture-project"),
                thread: &super::super::super::super::history::CodexThreadMetadata {
                    model_provider: "openai".to_string(),
                    model: "fixture-model".to_string(),
                    session_id: "fixture-parent-session".to_string(),
                    turn_id: "fixture-turn".to_string(),
                    last_assistant_message: "fixture final response".to_string(),
                },
                prompt: "classify this stopped task",
                result_path: Path::new("/private/tmp/status-worker-fixture/result.json"),
                settings_path: Path::new("/private/tmp/status-worker-fixture/srt-settings.json"),
                base_url: "http://localhost:43817/fixture-token/backend-api/codex",
                inference_dir: Path::new("/private/tmp/status-worker-fixture"),
                private_home: &private_home,
            },
            Path::new("/tools/codex-fixture"),
            Command::new("fixture-sandbox-runtime"),
        )?;
        let actual = CommandObservation {
            program: command.get_program().to_string_lossy().into_owned(),
            args: command
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect(),
            current_dir: command
                .get_current_dir()
                .map(|path| path.to_string_lossy().into_owned()),
            env: command
                .get_envs()
                .map(|(name, value)| {
                    (
                        name.to_string_lossy().into_owned(),
                        value.map(|value| value.to_string_lossy().into_owned()),
                    )
                })
                .collect(),
        };

        assert_eq!(
            actual,
            CommandObservation {
                program: "fixture-sandbox-runtime".to_string(),
                args: vec![
                    "--settings",
                    "/private/tmp/status-worker-fixture/srt-settings.json",
                    "/tools/codex-fixture",
                    "exec",
                    "fork",
                    "--ephemeral",
                    "--model",
                    "fixture-model",
                    "--config",
                    "model_provider=\"openai\"",
                    "--config",
                    "sqlite_home=\"/private/tmp/status-worker-fixture/codex-home/state\"",
                    "--config",
                    "log_dir=\"/private/tmp/status-worker-fixture/codex-home/logs\"",
                    "--config",
                    "cli_auth_credentials_store=\"file\"",
                    "--config",
                    "mcp_oauth_credentials_store=\"file\"",
                    "--config",
                    "openai_base_url=\"http://localhost:43817/fixture-token/backend-api/codex\"",
                    "--config",
                    "features.hooks=false",
                    "--config",
                    "features.enable_request_compression=false",
                    "--output-last-message",
                    "/private/tmp/status-worker-fixture/result.json",
                    "--",
                    "fixture-parent-session",
                    "classify this stopped task",
                ]
                .into_iter()
                .map(str::to_string)
                .collect(),
                current_dir: Some("/work/fixture-project".to_string()),
                env: BTreeMap::from([
                    ("ARMYKNIFE_SKIP_HOOKS".to_string(), Some("1".to_string())),
                    (
                        "CLAUDE_CODE_TMPDIR".to_string(),
                        Some("/private/tmp/status-worker-fixture".to_string()),
                    ),
                    (
                        "CLAUDE_TMPDIR".to_string(),
                        Some("/private/tmp/status-worker-fixture".to_string()),
                    ),
                    (
                        "CODEX_HOME".to_string(),
                        Some("/private/tmp/status-worker-fixture/codex-home".to_string()),
                    ),
                    (
                        "CODEX_SQLITE_HOME".to_string(),
                        Some("/private/tmp/status-worker-fixture/codex-home/state".to_string()),
                    ),
                    (
                        "TMPDIR".to_string(),
                        Some("/private/tmp/status-worker-fixture/codex-home/tmp".to_string()),
                    ),
                    (
                        "TMP".to_string(),
                        Some("/private/tmp/status-worker-fixture/codex-home/tmp".to_string()),
                    ),
                    (
                        "TEMP".to_string(),
                        Some("/private/tmp/status-worker-fixture/codex-home/tmp".to_string()),
                    ),
                    ("ARMYKNIFE_SESSION_ID".to_string(), None),
                    ("ARMYKNIFE_ANCESTOR_SESSION_IDS".to_string(), None),
                    ("TQ_SESSION_ID".to_string(), None),
                    ("TQ_PARENT_SESSION_ID".to_string(), None),
                    ("CLAUDE_CODE_SESSION_ID".to_string(), None),
                    ("CODEX_SESSION_ID".to_string(), None),
                    ("CODEX_THREAD_ID".to_string(), None),
                    ("OPENAI_API_KEY".to_string(), None),
                    ("OPENAI_BASE_URL".to_string(), None),
                    ("CODEX_API_KEY".to_string(), None),
                    ("CODEX_ACCESS_TOKEN".to_string(), None),
                ]),
            },
        );
        Ok(())
    }

    #[derive(Debug, PartialEq, Eq)]
    struct CommandObservation {
        program: String,
        args: Vec<String>,
        current_dir: Option<String>,
        env: BTreeMap<String, Option<String>>,
    }
}
