use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use regex::Regex;

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
const MAX_FORK_STDERR_BYTES: usize = 4 * 1024;

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
    with_fork_resources(
        |inference_path| home::prepare_private_codex_home(inference_path, session, thread),
        || relay::Relay::start(&thread.session_id),
        |inference_path, private_home, relay| {
            let settings_path = write_srt_settings(inference_path, relay.port())?;
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
                    inference_dir: inference_path,
                    private_home,
                },
                &codex_path,
                ExternalTool::SandboxRuntime.command(),
            )?;
            let output = process::run_in_process_group_with_timeout_and_stderr(
                command,
                FORK_TIMEOUT,
                MAX_FORK_STDERR_BYTES,
            )
            .context("Codex status fork process failed")?;
            let stderr = output.stderr;
            let status = match output.status {
                Ok(status) => status,
                Err(error) => bail!(
                    "{}",
                    fork_process_error_message(&error, &stderr, relay.credential())
                ),
            };
            ensure!(
                status.success(),
                "{}",
                fork_failure_message(status.code(), &stderr, relay.credential())
            );

            let result = read_result(&result_path)?;
            parse_configured_status_response(&result, configured_kinds)
        },
    )
}

fn fork_failure_message(status_code: Option<i32>, stderr: &[u8], relay_credential: &str) -> String {
    append_stderr(
        format!("Codex status fork exited with code {status_code:?}"),
        stderr,
        relay_credential,
    )
}

fn fork_process_error_message(
    error: &std::io::Error,
    stderr: &[u8],
    relay_credential: &str,
) -> String {
    append_stderr(
        format!("Codex status fork process failed: {error}"),
        stderr,
        relay_credential,
    )
}

fn append_stderr(mut message: String, stderr: &[u8], relay_credential: &str) -> String {
    if stderr.is_empty() {
        return message;
    }

    let stderr = sanitize_fork_stderr(stderr, relay_credential);
    message.push('\n');
    message.push_str(&indoc::formatdoc! {"
        Captured child stderr (max {MAX_FORK_STDERR_BYTES} bytes):
        {stderr}"
    });
    message
}

fn sanitize_fork_stderr(stderr: &[u8], relay_credential: &str) -> String {
    static AUTHORIZATION_PATTERN: OnceLock<Option<Regex>> = OnceLock::new();
    static CREDENTIAL_PATTERN: OnceLock<Option<Regex>> = OnceLock::new();
    static BEARER_PATTERN: OnceLock<Option<Regex>> = OnceLock::new();
    static JWT_PATTERN: OnceLock<Option<Regex>> = OnceLock::new();
    static API_TOKEN_PATTERN: OnceLock<Option<Regex>> = OnceLock::new();
    static OPAQUE_TOKEN_PATTERN: OnceLock<Option<Regex>> = OnceLock::new();

    let Some(authorization_pattern) = AUTHORIZATION_PATTERN
        .get_or_init(|| Regex::new(r#"(?im)(authorization["']?\s*[:=]\s*)[^\r\n]+"#).ok())
    else {
        return "[stderr omitted because credential redaction is unavailable]".to_string();
    };
    let Some(credential_pattern) = CREDENTIAL_PATTERN.get_or_init(|| {
        Regex::new(
            r#"(?i)([\"']?(?:access[_-]?token|refresh[_-]?token|id[_-]?token|api\s*[_-]?\s*key|token|secret)[\"']?\s*[:=]\s*[\"']?)([^\"'\s,;}\]]+)"#,
        )
        .ok()
    }) else {
        return "[stderr omitted because credential redaction is unavailable]".to_string();
    };
    let Some(bearer_pattern) =
        BEARER_PATTERN.get_or_init(|| Regex::new(r#"(?i)(bearer\s+)([^\s,;'\"]+)"#).ok())
    else {
        return "[stderr omitted because credential redaction is unavailable]".to_string();
    };
    let Some(jwt_pattern) = JWT_PATTERN.get_or_init(|| {
        Regex::new(r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b").ok()
    }) else {
        return "[stderr omitted because credential redaction is unavailable]".to_string();
    };
    let Some(api_token_pattern) = API_TOKEN_PATTERN.get_or_init(|| {
        Regex::new(r"\b(?:sk-[A-Za-z0-9_-]{12,}|gh[pousr]_[A-Za-z0-9_]{12,})\b").ok()
    }) else {
        return "[stderr omitted because credential redaction is unavailable]".to_string();
    };
    let Some(opaque_token_pattern) =
        OPAQUE_TOKEN_PATTERN.get_or_init(|| Regex::new(r"\b[A-Za-z0-9_-]{32,}\b").ok())
    else {
        return "[stderr omitted because credential redaction is unavailable]".to_string();
    };

    let stderr = String::from_utf8_lossy(stderr);
    let stderr = authorization_pattern.replace_all(&stderr, "$1[REDACTED]");
    let stderr = bearer_pattern.replace_all(&stderr, "$1[REDACTED]");
    let stderr = credential_pattern.replace_all(&stderr, "$1[REDACTED]");
    let stderr = jwt_pattern.replace_all(&stderr, "[REDACTED]");
    let stderr = api_token_pattern.replace_all(&stderr, "[REDACTED]");
    let stderr = opaque_token_pattern.replace_all(&stderr, "[REDACTED]");
    if relay_credential.is_empty() {
        stderr.into_owned()
    } else {
        stderr.replace(relay_credential, "[REDACTED]")
    }
}

fn with_fork_resources<T, H, L>(
    prepare_private_home: impl FnOnce(&Path) -> Result<H>,
    start_relay: impl FnOnce() -> Result<L>,
    run_fork: impl FnOnce(&Path, &H, &L) -> Result<T>,
) -> Result<T> {
    let inference_dir = tempfile::Builder::new()
        .prefix("armyknife-codex-status-")
        .tempdir()
        .context("create Codex status inference directory")?;
    let inference_path = std::fs::canonicalize(inference_dir.path())
        .context("resolve Codex status inference directory")?;
    let private_home = prepare_private_home(&inference_path)?;
    let relay = start_relay()?;
    let result = run_fork(&inference_path, &private_home, &relay);
    drop(relay);
    drop(inference_dir);
    result
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
    use std::io;
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use indoc::indoc;
    use rstest::rstest;
    use serde_json::json;

    use super::{
        fork_failure_message, fork_process_error_message, infer_status, with_fork_resources,
    };
    use crate::commands::agent::codex_steer::history::CodexThreadMetadata;
    use crate::commands::agent::codex_steer::status_inference::relay::Relay;
    use crate::commands::agent::types::Session;

    #[test]
    fn fork_failure_message_includes_stderr_after_redacting_credentials() {
        let actual = fork_failure_message(
            Some(1),
            indoc! {
                r#"
                Authorization: Basic fixture-basic-credential
                Authorization: Bearer fixture-bearer-token
                {"access_token":"fixture-access-token"}
                request URL: http://localhost:43817/fixture-relay-token/backend-api/codex
                response: eyJfixtureheader.fixturepayload.fixturesignature
                api key: sk-fixture-token-that-is-long-enough
                "#
            }
            .as_bytes(),
            "fixture-relay-token",
        );

        assert_eq!(
            actual,
            concat!(
                "Codex status fork exited with code Some(1)\n",
                "Captured child stderr (max 4096 bytes):\n",
                "Authorization: [REDACTED]\n",
                "Authorization: [REDACTED]\n",
                "{\"access_token\":\"[REDACTED]\"}\n",
                "request URL: http://localhost:43817/[REDACTED]/backend-api/codex\n",
                "response: [REDACTED]\n",
                "api key: [REDACTED]\n",
            ),
        );
    }

    #[test]
    fn fork_process_error_message_includes_stderr() {
        let error = io::Error::new(io::ErrorKind::TimedOut, "command timed out after 120s");
        let actual = fork_process_error_message(&error, b"fixture timeout detail", "");

        assert_eq!(
            actual,
            concat!(
                "Codex status fork process failed: command timed out after 120s\n",
                "Captured child stderr (max 4096 bytes):\n",
                "fixture timeout detail",
            ),
        );
    }

    #[derive(Clone, Copy)]
    enum ForkOutcome {
        Success,
        Failure,
        Timeout,
        Interrupted,
    }

    #[test]
    fn fork_rejects_non_openai_provider_before_starting_inference() {
        let session: Session = serde_json::from_value(json!({
            "session_id": "fixture-session",
            "cwd": "/workspace/fixture",
            "transcript_path": null,
            "tmux_info": null,
            "status": "stopped",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_message": null,
        }))
        .expect("fixture session should deserialize");
        let thread = CodexThreadMetadata {
            model_provider: "custom".to_string(),
            model: "fixture-model".to_string(),
            session_id: "fixture-thread".to_string(),
            turn_id: "fixture-turn".to_string(),
            last_assistant_message: "fixture final message".to_string(),
        };

        let actual = infer_status(&session, &thread, "fixture prompt", &[])
            .map_err(|error| format!("{error:#}"));

        assert_eq!(
            actual,
            Err("Codex status fork requires the built-in OpenAI provider".to_string())
        );
    }

    #[rstest]
    #[case::success(ForkOutcome::Success, Ok(()))]
    #[case::failure(ForkOutcome::Failure, Err(Some(io::ErrorKind::Other)))]
    #[case::timeout(ForkOutcome::Timeout, Err(Some(io::ErrorKind::TimedOut)))]
    #[case::interruption(ForkOutcome::Interrupted, Err(Some(io::ErrorKind::Interrupted)))]
    #[tokio::test]
    async fn fork_outcomes_release_private_home_and_relay(
        #[case] outcome: ForkOutcome,
        #[case] expected_result: Result<(), Option<io::ErrorKind>>,
    ) {
        let private_home_path = Arc::new(Mutex::new(None::<PathBuf>));
        let relay_port = Arc::new(Mutex::new(None::<u16>));
        let captured_private_home = private_home_path.clone();
        let captured_relay_port = relay_port.clone();
        let result = with_fork_resources(
            move |inference_path| {
                let private_home = inference_path.join("codex-home");
                std::fs::create_dir_all(&private_home)?;
                std::fs::write(private_home.join("fixture"), b"private")?;
                *captured_private_home
                    .lock()
                    .map_err(|_| anyhow::anyhow!("private-home fixture lock was poisoned"))? =
                    Some(private_home.clone());
                Ok(private_home)
            },
            move || {
                let relay = Relay::start("fixture-parent-session")?;
                *captured_relay_port
                    .lock()
                    .map_err(|_| anyhow::anyhow!("relay fixture lock was poisoned"))? =
                    Some(relay.port());
                Ok(relay)
            },
            |_, _, _| match outcome {
                ForkOutcome::Success => Ok(()),
                ForkOutcome::Failure => Err(io::Error::other("fixture fork failed").into()),
                ForkOutcome::Timeout => {
                    Err(io::Error::new(io::ErrorKind::TimedOut, "fixture fork timed out").into())
                }
                ForkOutcome::Interrupted => Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "fixture fork was interrupted",
                )
                .into()),
            },
        );
        let result = match result {
            Ok(()) => Ok(()),
            Err(error) => Err(error.downcast_ref::<io::Error>().map(io::Error::kind)),
        };
        let private_home_path = private_home_path
            .lock()
            .expect("private-home fixture path should be recorded")
            .clone()
            .expect("private-home fixture path should be available");
        let relay_port = relay_port
            .lock()
            .expect("relay fixture port should be recorded")
            .to_owned()
            .expect("relay fixture port should be available");
        let relay_released = TcpListener::bind(("127.0.0.1", relay_port))
            .map(|listener| {
                drop(listener);
                true
            })
            .unwrap_or(false);

        assert_eq!(
            (result, private_home_path.exists(), relay_released),
            (expected_result, false, true),
        );
    }

    #[test]
    fn private_home_preparation_failure_does_not_start_the_relay() {
        let relay_started = Arc::new(AtomicBool::new(false));
        let fork_started = Arc::new(AtomicBool::new(false));
        let inference_path = Arc::new(Mutex::new(None::<PathBuf>));
        let captured_relay_started = relay_started.clone();
        let captured_fork_started = fork_started.clone();
        let captured_inference_path = inference_path.clone();
        let result: anyhow::Result<()> = with_fork_resources(
            move |path| -> anyhow::Result<()> {
                *captured_inference_path
                    .lock()
                    .map_err(|_| anyhow::anyhow!("inference-path fixture lock was poisoned"))? =
                    Some(path.to_path_buf());
                Err(anyhow::anyhow!("fixture private-home preparation failed"))
            },
            move || {
                captured_relay_started.store(true, Ordering::Relaxed);
                Ok(())
            },
            move |_, _, _| {
                captured_fork_started.store(true, Ordering::Relaxed);
                Ok(())
            },
        );

        assert_eq!(
            (
                result.map(|_| ()).map_err(|_| ()),
                relay_started.load(Ordering::Relaxed),
                fork_started.load(Ordering::Relaxed),
                inference_path
                    .lock()
                    .expect("inference path should be recorded")
                    .as_ref()
                    .is_some_and(|path| path.exists()),
            ),
            (Err(()), false, false, false),
        );
    }
}
