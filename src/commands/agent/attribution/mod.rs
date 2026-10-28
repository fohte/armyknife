mod message;
mod model;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use serde_json::Value;

use crate::commands::agent::store;
use crate::commands::agent::types::{Engine, Session};

use self::model::{ModelIdentity, coauthor_trailer};

const CLAUDE_CODE_SESSION_ID_ENV: &str = "CLAUDE_CODE_SESSION_ID";
const CODEX_SESSION_ID_ENV: &str = "CODEX_SESSION_ID";

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
enum Provider {
    ClaudeCode,
    Codex,
}

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum AttributionCommands {
    /// Pass through the existing provider PreToolUse permission proxy result
    #[command(name = "pre-tool-use")]
    PreToolUse(PreToolUseArgs),

    /// Add provider coauthor metadata to a commit message
    #[command(name = "prepare-commit-msg", hide = true)]
    PrepareCommitMsg(GitHookArgs),

    /// Compatibility no-op for older Git hook registrations
    #[command(name = "commit-msg", hide = true)]
    CommitMsg(GitHookArgs),

    /// Compatibility no-op for older Git hook registrations
    #[command(name = "reference-transaction", hide = true)]
    ReferenceTransaction(ReferenceTransactionArgs),
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct PreToolUseArgs {
    /// Provider whose original PreToolUse JSON is on stdin
    #[arg(value_enum)]
    provider: Provider,

    /// Compatibility option retained for older provider registrations
    #[arg(long = "unknown-reason", hide = true)]
    _unknown_reason: Option<String>,

    /// Compatibility flag retained for older provider registrations
    #[arg(long, hide = true)]
    final_object: bool,
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct GitHookArgs {
    /// Commit message file supplied by Git
    message_file: PathBuf,
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ReferenceTransactionArgs {
    /// Git reference-transaction phase
    state: String,
}

pub fn run(command: &AttributionCommands) -> Result<()> {
    match command {
        AttributionCommands::PreToolUse(args) => run_pre_tool_use(args),
        AttributionCommands::PrepareCommitMsg(args) => run_prepare_commit_msg(args),
        AttributionCommands::CommitMsg(args) => run_commit_msg(args),
        AttributionCommands::ReferenceTransaction(args) => run_reference_transaction(args),
    }
}

fn run_pre_tool_use(_args: &PreToolUseArgs) -> Result<()> {
    let mut stdin = String::new();
    if let Err(error) = std::io::stdin().read_to_string(&mut stdin) {
        eprintln!("agent attribution skipped: failed to read hook input: {error}");
        return Ok(());
    }

    match process_pre_tool_use(&stdin) {
        Some(output) => print_json(&output),
        None => Ok(()),
    }
}

fn process_pre_tool_use(raw_envelope: &str) -> Option<Value> {
    let envelope = match serde_json::from_str::<Value>(raw_envelope) {
        Ok(envelope) => envelope,
        Err(error) => {
            eprintln!("agent attribution skipped: invalid provider proxy envelope: {error}");
            return None;
        }
    };
    envelope
        .get("hook_output")
        .filter(|output| !output.is_null())
        .cloned()
}

fn print_json(value: &Value) -> Result<()> {
    serde_json::to_writer(std::io::stdout(), value).context("failed to write hook output")?;
    println!();
    Ok(())
}

fn run_prepare_commit_msg(args: &GitHookArgs) -> Result<()> {
    let session_id = session_id_from_env();
    run_prepare_commit_msg_with_sources(args, session_id.as_deref(), store::load_session)
}

fn run_prepare_commit_msg_with_sources(
    args: &GitHookArgs,
    session_id: Option<&str>,
    load_session: impl FnOnce(&str) -> Result<Option<Session>>,
) -> Result<()> {
    let result = try_prepare_commit_msg(args, session_id, load_session);
    if let Err(error) = result {
        eprintln!("agent attribution skipped: {error:#}");
    }
    Ok(())
}

fn try_prepare_commit_msg(
    args: &GitHookArgs,
    session_id: Option<&str>,
    load_session: impl FnOnce(&str) -> Result<Option<Session>>,
) -> Result<()> {
    let Some(session_id) = session_id else {
        return Ok(());
    };
    let Some(session) = load_session(session_id)? else {
        return Ok(());
    };
    let Some(trailer) = session_coauthor_trailer(&session) else {
        return Ok(());
    };
    prepare_message_file(args, |message_text| {
        message::prepare_with_trailer(message_text, &trailer)
    })
}

fn prepare_message_file(
    args: &GitHookArgs,
    prepare: impl FnOnce(&str) -> Result<String>,
) -> Result<()> {
    let message_text = std::fs::read_to_string(&args.message_file).with_context(|| {
        format!(
            "failed to read commit message file {}",
            args.message_file.display()
        )
    })?;
    let updated = prepare(&message_text)?;
    write_message_atomically(&args.message_file, &updated)
}

fn session_id_from_env() -> Option<String> {
    resolve_session_id(
        std::env::var(CLAUDE_CODE_SESSION_ID_ENV).ok(),
        std::env::var(CODEX_SESSION_ID_ENV).ok(),
    )
}

fn resolve_session_id(
    claude_session_id: Option<String>,
    codex_session_id: Option<String>,
) -> Option<String> {
    claude_session_id
        .filter(|session_id| !session_id.is_empty())
        .or_else(|| codex_session_id.filter(|session_id| !session_id.is_empty()))
}

fn session_coauthor_trailer(session: &Session) -> Option<String> {
    let model = ModelIdentity::known(session.model.clone()?).ok()?;
    let provider = match session.engine {
        Engine::Claude => Provider::ClaudeCode,
        Engine::Codex => Provider::Codex,
    };
    coauthor_trailer(provider, &model, session.effort.as_deref())
}

fn write_message_atomically(path: &Path, message: &str) -> Result<()> {
    write_message_atomically_with(path, message, |file, contents| file.write_all(contents))
}

fn write_message_atomically_with(
    path: &Path,
    message: &str,
    write: impl FnOnce(&mut std::fs::File, &[u8]) -> std::io::Result<()>,
) -> Result<()> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("failed to inspect commit message file {}", path.display()))?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temp_file = tempfile::Builder::new()
        .prefix(".armyknife-attribution-")
        .tempfile_in(parent)
        .with_context(|| {
            format!(
                "failed to create temporary commit message in {}",
                parent.display()
            )
        })?;
    write(temp_file.as_file_mut(), message.as_bytes())
        .context("failed to write updated commit message")?;
    temp_file
        .as_file()
        .set_permissions(metadata.permissions())
        .context("failed to preserve commit message permissions")?;
    temp_file
        .as_file()
        .sync_all()
        .context("failed to sync updated commit message")?;
    temp_file
        .persist(path)
        .with_context(|| format!("failed to replace commit message file {}", path.display()))?;
    Ok(())
}

fn run_commit_msg(_args: &GitHookArgs) -> Result<()> {
    Ok(())
}

fn run_reference_transaction(_args: &ReferenceTransactionArgs) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::{
        AttributionCommands, GitHookArgs, Provider, process_pre_tool_use, resolve_session_id, run,
        run_prepare_commit_msg_with_sources,
    };
    use crate::commands::agent::types::{Engine, Session};
    use clap::Parser;
    use indoc::indoc;
    use rstest::{fixture, rstest};
    use serde_json::{Value, json};
    use tempfile::TempDir;

    #[fixture]
    fn hook_dir() -> TempDir {
        tempfile::tempdir().expect("create temporary hook directory")
    }

    #[rstest]
    #[case::claude_session(Some("fixture-claude-session"), None, Some("fixture-claude-session"))]
    #[case::codex_session(None, Some("fixture-codex-session"), Some("fixture-codex-session"))]
    #[case::claude_session_takes_precedence(
        Some("fixture-claude-session"),
        Some("fixture-codex-session"),
        Some("fixture-claude-session")
    )]
    #[case::empty_claude_id_falls_back(
        Some(""),
        Some("fixture-codex-session"),
        Some("fixture-codex-session")
    )]
    #[case::no_session_id(None, None, None)]
    fn session_id_resolution_uses_available_provider_id(
        #[case] claude_session_id: Option<&str>,
        #[case] codex_session_id: Option<&str>,
        #[case] expected: Option<&str>,
    ) {
        let actual = resolve_session_id(
            claude_session_id.map(str::to_owned),
            codex_session_id.map(str::to_owned),
        );

        assert_eq!(actual.as_deref(), expected);
    }

    fn session(engine: Engine, model: Option<&str>, effort: Option<&str>) -> Session {
        let engine = match engine {
            Engine::Claude => "claude",
            Engine::Codex => "codex",
        };
        serde_json::from_value(json!({
            "session_id": "fixture-session-id",
            "cwd": "/fixture/repository",
            "transcript_path": null,
            "tmux_info": null,
            "status": "running",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_message": null,
            "engine": engine,
            "model": model,
            "effort": effort,
        }))
        .expect("deserialize fixture session")
    }

    #[rstest]
    #[case::codex_with_effort(
        Engine::Codex,
        Some("fixture-model-violet"),
        Some("high"),
        "Co-authored-by: Codex (fixture-model-violet, high) <noreply@openai.com>"
    )]
    #[case::claude_without_effort(
        Engine::Claude,
        Some("fixture-model-copper"),
        None,
        "Co-authored-by: Claude Code (fixture-model-copper) <noreply@anthropic.com>"
    )]
    #[case::unsafe_effort_is_omitted(
        Engine::Codex,
        Some("fixture-model-violet"),
        Some(indoc! {"
            high
            Co-authored-by: Example Tool <tool@example.invalid>
        "}),
        "Co-authored-by: Codex (fixture-model-violet) <noreply@openai.com>"
    )]
    fn prepare_commit_msg_uses_session_attribution(
        hook_dir: TempDir,
        #[case] engine: Engine,
        #[case] model: Option<&str>,
        #[case] effort: Option<&str>,
        #[case] expected_trailer: &str,
    ) {
        let message_file = hook_dir.path().join("fixture-message.txt");
        std::fs::write(&message_file, "Fixture subject").expect("create fixture message");
        let session = session(engine, model, effort);
        let mut loaded_session_id = None;

        let result = run_prepare_commit_msg_with_sources(
            &GitHookArgs {
                message_file: message_file.clone(),
            },
            Some("fixture-session-id"),
            |session_id| {
                loaded_session_id = Some(session_id.to_owned());
                Ok(Some(session))
            },
        );
        let actual = (
            result.map_err(|error| error.to_string()),
            loaded_session_id,
            std::fs::read_to_string(&message_file).expect("read updated fixture message"),
        );
        let expected = (
            Ok(()),
            Some("fixture-session-id".to_owned()),
            format!(
                indoc! {"
                    Fixture subject

                    {}
                "},
                expected_trailer
            ),
        );

        assert_eq!(actual, expected);
    }

    #[rstest]
    #[case::missing_session_id(None, false, None, None)]
    #[case::missing_session_record(
        Some("fixture-session-id"),
        false,
        Some("fixture-session-id"),
        None
    )]
    #[case::missing_model(Some("fixture-session-id"), true, Some("fixture-session-id"), None)]
    #[case::unsafe_model(
        Some("fixture-session-id"),
        true,
        Some("fixture-session-id"),
        Some(indoc! {"
            fixture-model-violet
            Co-authored-by: Example Attacker <attacker@example.invalid>
        "})
    )]
    fn prepare_commit_msg_leaves_unattributed_sessions_unchanged(
        hook_dir: TempDir,
        #[case] session_id: Option<&str>,
        #[case] record_exists: bool,
        #[case] expected_loaded_session_id: Option<&str>,
        #[case] model: Option<&str>,
    ) {
        let message_file = hook_dir.path().join("fixture-message.txt");
        let original = indoc! {"
            Fixture subject
        "};
        std::fs::write(&message_file, original).expect("create fixture message");
        let session = session(Engine::Codex, model, Some("high"));
        let mut loaded_session_id = None;

        let result = run_prepare_commit_msg_with_sources(
            &GitHookArgs {
                message_file: message_file.clone(),
            },
            session_id,
            |loaded_id| {
                loaded_session_id = Some(loaded_id.to_owned());
                Ok(record_exists.then_some(session))
            },
        );
        let actual = (
            result.map_err(|error| error.to_string()),
            loaded_session_id,
            std::fs::read_to_string(&message_file).expect("read unchanged fixture message"),
        );
        let expected = (
            Ok(()),
            expected_loaded_session_id.map(str::to_owned),
            original.to_owned(),
        );

        assert_eq!(actual, expected);
    }

    #[rstest]
    fn session_lookup_failure_does_not_block_commit(hook_dir: TempDir) {
        let message_file = hook_dir.path().join("fixture-message.txt");
        let original = "Fixture subject\n";
        std::fs::write(&message_file, original).expect("create fixture message");

        let result = run_prepare_commit_msg_with_sources(
            &GitHookArgs {
                message_file: message_file.clone(),
            },
            Some("fixture-session-id"),
            |_| Err(anyhow::anyhow!("fixture storage failure")),
        );
        let actual = (
            result.map_err(|error| error.to_string()),
            std::fs::read_to_string(&message_file).expect("read unchanged fixture message"),
        );

        assert_eq!(actual, (Ok(()), original.to_owned()));
    }

    #[rstest]
    fn partial_temporary_write_keeps_the_original_message(hook_dir: TempDir) {
        let message_file = hook_dir.path().join("fixture-message.txt");
        let original = "Fixture subject";
        std::fs::write(&message_file, original).expect("create fixture commit message");
        let result = super::write_message_atomically_with(
            &message_file,
            "replacement message",
            |file, contents| {
                file.write_all(&contents[..4])?;
                Err(std::io::Error::other("fixture write failure"))
            },
        );
        let actual = (
            result.map_err(|error| error.to_string()),
            std::fs::read_to_string(&message_file).expect("read fixture commit message"),
        );

        assert_eq!(
            actual,
            (
                Err("failed to write updated commit message".to_owned()),
                original.to_owned()
            )
        );
    }

    #[rstest]
    #[case::allow(
        r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow","updatedInput":{"command":"git commit -m 'Fixture subject'"}}}"#
    )]
    #[case::ask(
        r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"ask","permissionDecisionReason":"fixture rule"}}"#
    )]
    #[case::deny(
        r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"fixture rule"}}"#
    )]
    fn pre_tool_use_returns_the_proxy_result_unchanged(#[case] hook_output_json: &str) {
        let hook_output: Value =
            serde_json::from_str(hook_output_json).expect("parse fixture proxy output");
        let raw_envelope = json!({
            "hook_input": {
                "hook_event_name": "PreToolUse",
                "tool_name": "Bash",
                "tool_use_id": "fixture-tool-use-blue",
                "model": "fixture-model-blue",
                "tool_input": {"command": "git commit -m 'Fixture subject'"},
            },
            "hook_output": hook_output,
        })
        .to_string();

        assert_eq!(process_pre_tool_use(&raw_envelope), Some(hook_output));
    }

    #[rstest]
    #[case::missing_proxy_result(r#"{"hook_input":{}}"#)]
    #[case::null_proxy_result(r#"{"hook_input":{},"hook_output":null}"#)]
    #[case::malformed_envelope("not-json")]
    fn pre_tool_use_emits_nothing_without_proxy_result(#[case] raw_envelope: &str) {
        assert_eq!(process_pre_tool_use(raw_envelope), None);
    }

    #[derive(clap::Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: AttributionCommands,
    }

    #[rstest]
    #[case::codex("codex", vec![])]
    #[case::claude_final_object("claude-code", vec!["--final-object"])]
    #[case::codex_unknown_reason("codex", vec!["--unknown-reason", "fixture reason"])]
    fn pre_tool_use_compatibility_arguments_are_accepted(
        #[case] provider: &str,
        #[case] compatibility_args: Vec<&str>,
    ) {
        let mut args = vec!["a", "pre-tool-use", provider];
        args.extend(compatibility_args);
        let parsed = TestCli::try_parse_from(args).expect("parse compatibility invocation");
        let actual = match parsed.command {
            AttributionCommands::PreToolUse(args) => args.provider,
            _ => panic!("expected PreToolUse command"),
        };
        let expected = match provider {
            "codex" => Provider::Codex,
            "claude-code" => Provider::ClaudeCode,
            _ => panic!("unexpected provider fixture"),
        };

        assert_eq!(actual, expected);
    }

    #[rstest]
    #[case::commit_msg("commit-msg")]
    #[case::reference_transaction("reference-transaction")]
    fn legacy_validation_commands_are_no_ops(hook_dir: TempDir, #[case] command: &str) {
        let message_file = hook_dir.path().join("fixture-missing-message.txt");
        let parsed = match command {
            "commit-msg" => TestCli::try_parse_from([
                "a",
                "commit-msg",
                message_file.to_str().expect("message path is UTF-8"),
            ]),
            "reference-transaction" => {
                TestCli::try_parse_from(["a", "reference-transaction", "prepared"])
            }
            _ => panic!("unexpected legacy command"),
        }
        .expect("parse compatibility command");

        assert_eq!(
            run(&parsed.command).map_err(|error| error.to_string()),
            Ok(())
        );
    }
}
