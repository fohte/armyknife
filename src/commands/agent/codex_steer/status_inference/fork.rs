use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

use super::super::history::CodexThreadMetadata;
use super::relay;
use crate::commands::agent::status_inference::parse_configured_status_response;
use crate::commands::agent::types::{AgentStatus, Session};
use crate::infra::external_tool::ExternalTool;
use crate::infra::process;

const FORK_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_RESULT_BYTES: u64 = 64 * 1024;

pub(super) fn infer_status(
    session: &Session,
    thread: &CodexThreadMetadata,
    prompt: &str,
    configured_kinds: &[String],
) -> Result<AgentStatus> {
    let inference_dir = tempfile::Builder::new()
        .prefix("armyknife-codex-status-")
        .tempdir()
        .context("create Codex status inference directory")?;
    let inference_path = std::fs::canonicalize(inference_dir.path())
        .context("resolve Codex status inference directory")?;
    let relay = relay::Relay::start(&thread.session_id)?;
    let settings_path = write_srt_settings(&inference_path, relay.port())?;
    let result_path = inference_path.join("result.json");
    let command = codex_fork_command(
        session,
        thread,
        prompt,
        &result_path,
        &settings_path,
        &relay.base_url(),
        &inference_path,
    )?;
    let status = process::run_in_process_group_with_timeout(command, FORK_TIMEOUT)
        .context("Codex status fork process failed")?;
    ensure!(
        status.success(),
        "Codex status fork exited with code {:?}",
        status.code()
    );

    let result = read_result(&result_path)?;
    let status = parse_configured_status_response(&result, configured_kinds)?;
    drop(relay);
    drop(inference_dir);
    Ok(status)
}

fn codex_fork_command(
    session: &Session,
    thread: &CodexThreadMetadata,
    prompt: &str,
    result_path: &Path,
    settings_path: &Path,
    base_url: &str,
    inference_dir: &Path,
) -> Result<Command> {
    let codex_path = ExternalTool::Codex
        .resolve_path()
        .context("Codex CLI is not available on PATH")?;
    let mut command = ExternalTool::SandboxRuntime.command();
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
        )?)
        .current_dir(&session.cwd)
        .env("ARMYKNIFE_SKIP_HOOKS", "1")
        .env("CLAUDE_CODE_TMPDIR", inference_dir)
        .env("CLAUDE_TMPDIR", inference_dir)
        .env_remove("OPENAI_API_KEY")
        .env_remove("CODEX_API_KEY")
        .env_remove("CODEX_ACCESS_TOKEN");
    Ok(command)
}

fn codex_fork_args(
    session_id: &str,
    model: &str,
    prompt: &str,
    result_path: &Path,
    base_url: &str,
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
        format!("openai_base_url={}", serde_json::to_string(base_url)?),
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

fn write_srt_settings(inference_dir: &Path, relay_port: u16) -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is unavailable for Codex status sandbox settings")?;
    let settings = srt_settings(inference_dir, relay_port, &home)?;
    let path = inference_dir.join("srt-settings.json");
    std::fs::write(&path, serde_json::to_vec(&settings)?)
        .context("write Codex status sandbox settings")?;
    Ok(path)
}

fn srt_settings(inference_dir: &Path, relay_port: u16, home: &Path) -> Result<Value> {
    let inference_dir = path_to_string(inference_dir)?;
    // srt always appends these compatibility paths to filesystem.allowWrite.
    let deny_write = [
        Path::new("/dev/stdout").to_path_buf(),
        Path::new("/dev/stderr").to_path_buf(),
        Path::new("/dev/null").to_path_buf(),
        Path::new("/dev/tty").to_path_buf(),
        Path::new("/dev/dtracehelper").to_path_buf(),
        Path::new("/dev/autofs_nowait").to_path_buf(),
        Path::new("/tmp/claude").to_path_buf(),
        Path::new("/private/tmp/claude").to_path_buf(),
        home.join(".npm/_logs"),
        home.join(".claude/debug"),
    ]
    .into_iter()
    .map(|path| path_to_string(&path))
    .collect::<Result<Vec<_>>>()?;
    Ok(json!({
        "network": {
            "allowedDomains": [],
            "deniedDomains": ["*"],
            "strictAllowlist": true,
            "allowLocalBinding": false,
            "httpProxyPort": relay_port,
            "socksProxyPort": relay_port,
        },
        "filesystem": {
            "denyRead": [],
            "allowWrite": [inference_dir],
            "denyWrite": deny_write,
        },
    }))
}

fn path_to_string(path: &Path) -> Result<String> {
    path.to_str()
        .map(ToOwned::to_owned)
        .context("Codex status inference path is not valid UTF-8")
}

fn read_result(path: &Path) -> Result<String> {
    let metadata = std::fs::metadata(path).context("read Codex status fork result metadata")?;
    ensure!(
        metadata.len() <= MAX_RESULT_BYTES,
        "Codex status fork result exceeds {MAX_RESULT_BYTES} bytes"
    );
    std::fs::read_to_string(path).context("read Codex status fork result")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use serde_json::json;

    #[rstest]
    #[case::mac_tmp_fixture_path(
        Path::new("/private/tmp/status-worker-fixture"),
        Path::new("/Users/example"),
        43817,
        json!({
            "network": {
                "allowedDomains": [],
                "deniedDomains": ["*"],
                "strictAllowlist": true,
                "allowLocalBinding": false,
                "httpProxyPort": 43817,
                "socksProxyPort": 43817,
            },
            "filesystem": {
                "denyRead": [],
                "allowWrite": ["/private/tmp/status-worker-fixture"],
                "denyWrite": [
                    "/dev/stdout",
                    "/dev/stderr",
                    "/dev/null",
                    "/dev/tty",
                    "/dev/dtracehelper",
                    "/dev/autofs_nowait",
                    "/tmp/claude",
                    "/private/tmp/claude",
                    "/Users/example/.npm/_logs",
                    "/Users/example/.claude/debug",
                ],
            },
        }),
    )]
    fn srt_settings_limit_network_and_file_writes(
        #[case] inference_dir: &Path,
        #[case] home: &Path,
        #[case] relay_port: u16,
        #[case] expected: Value,
    ) {
        let actual = srt_settings(inference_dir, relay_port, home)
            .expect("fixture sandbox settings should be valid");
        assert_eq!(actual, expected);
    }

    #[test]
    fn codex_fork_uses_parent_model_ephemeral_history_and_relay_overrides() {
        let actual = codex_fork_args(
            "fixture-parent-session",
            "fixture-model",
            "classify this stopped task",
            Path::new("/private/tmp/status-worker-fixture/result.json"),
            "http://localhost:43817/fixture-token/backend-api/codex",
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
}
