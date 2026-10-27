use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, ensure};

use self::command::{ForkCommandOptions, codex_fork_command};
use self::sandbox::write_srt_settings;
use super::super::history::CodexThreadMetadata;
use super::relay;
use crate::commands::agent::status_inference::parse_configured_status_response;
use crate::commands::agent::types::{AgentStatus, Session};
use crate::infra::external_tool::ExternalTool;
use crate::infra::process;

mod command;
mod home;
mod sandbox;

const FORK_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_RESULT_BYTES: u64 = 64 * 1024;

pub(super) fn infer_status(
    session: &Session,
    thread: &CodexThreadMetadata,
    prompt: &str,
    configured_kinds: &[String],
) -> Result<AgentStatus> {
    ensure!(
        thread.model_provider == "openai",
        "Codex status fork requires the built-in OpenAI provider"
    );
    let inference_dir = tempfile::Builder::new()
        .prefix("armyknife-codex-status-")
        .tempdir()
        .context("create Codex status inference directory")?;
    let inference_path = std::fs::canonicalize(inference_dir.path())
        .context("resolve Codex status inference directory")?;
    let private_home = home::prepare_private_codex_home(&inference_path, session, thread)?;
    let relay = relay::Relay::start(&thread.session_id)?;
    let settings_path = write_srt_settings(&inference_path, relay.port())?;
    let result_path = inference_path.join("result.json");
    let codex_path = ExternalTool::Codex
        .resolve_path()
        .context("Codex CLI is not available on PATH")?;
    let command = codex_fork_command(
        ForkCommandOptions {
            cwd: &session.cwd,
            thread,
            prompt,
            result_path: &result_path,
            settings_path: &settings_path,
            base_url: &relay.base_url(),
            inference_dir: &inference_path,
            private_home: &private_home,
        },
        &codex_path,
        ExternalTool::SandboxRuntime.command(),
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
