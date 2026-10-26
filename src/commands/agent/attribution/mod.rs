mod command;
mod context;
mod hook;
mod message;
mod transcript;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use self::context::ModelIdentity;

const CONTEXT_ENV: &str = "ARMYKNIFE_AGENT_ATTRIBUTION_CONTEXT";

#[derive(Debug, Clone, Copy, Deserialize, Serialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum Provider {
    ClaudeCode,
    Codex,
}

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum AttributionCommands {
    /// Transform one provider PreToolUse request after the existing permission proxy
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

    /// Compatibility option; model attribution still requires runtime evidence
    #[arg(long, hide = true)]
    unknown_reason: Option<String>,

    /// Compatibility flag accepted from older provider registrations
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

fn run_pre_tool_use(args: &PreToolUseArgs) -> Result<()> {
    let mut stdin = String::new();
    if let Err(error) = std::io::stdin().read_to_string(&mut stdin) {
        eprintln!("agent attribution skipped: failed to read hook input: {error}");
        return Ok(());
    }

    match process_pre_tool_use(args, &stdin, &context::default_context_dir()) {
        Some(output) => print_json(&output),
        None => Ok(()),
    }
}

fn process_pre_tool_use(
    args: &PreToolUseArgs,
    raw_envelope: &str,
    context_dir: &Path,
) -> Option<Value> {
    let _ = args.final_object;
    match hook::transform(
        args.provider,
        raw_envelope,
        args.unknown_reason.as_deref(),
        context_dir,
    ) {
        Ok(output) => output,
        Err(error) => {
            eprintln!("agent attribution skipped: {error:#}");
            permission_output(raw_envelope)
        }
    }
}

fn permission_output(raw_envelope: &str) -> Option<Value> {
    serde_json::from_str::<Value>(raw_envelope)
        .ok()?
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
    let context_path = std::env::var_os(CONTEXT_ENV).map(PathBuf::from);
    run_prepare_commit_msg_with_context(args, context_path.as_deref())
}

fn run_prepare_commit_msg_with_context(
    args: &GitHookArgs,
    context_path: Option<&Path>,
) -> Result<()> {
    let result = try_prepare_commit_msg(args, context_path);
    if let Err(error) = result {
        eprintln!("agent attribution skipped: {error:#}");
    }
    Ok(())
}

fn try_prepare_commit_msg(args: &GitHookArgs, context_path: Option<&Path>) -> Result<()> {
    let Some(context_path) = context_path else {
        return Ok(());
    };
    let message_text = std::fs::read_to_string(&args.message_file).with_context(|| {
        format!(
            "failed to read commit message file {}",
            args.message_file.display()
        )
    })?;
    let updated = message::prepare(context_path, &message_text)?;
    write_message_atomically(&args.message_file, &updated)
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

fn model_identity(model: Option<String>) -> Result<ModelIdentity> {
    let Some(model) = model else {
        anyhow::bail!("provider runtime model is unavailable");
    };
    ModelIdentity::known(model)
}

#[derive(Debug, Deserialize)]
struct ProxyEnvelope {
    hook_input: Value,
    #[serde(default)]
    hook_output: Option<Value>,
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::PathBuf;

    use super::{
        AttributionCommands, CONTEXT_ENV, GitHookArgs, PreToolUseArgs, Provider,
        process_pre_tool_use, run,
    };
    use crate::commands::agent::attribution::context::{self, ModelIdentity, OperationContext};
    use clap::Parser;
    use indoc::indoc;
    use rstest::{fixture, rstest};
    use serde_json::{Value, json};
    use tempfile::TempDir;

    #[fixture]
    fn hook_dir() -> TempDir {
        tempfile::tempdir().expect("create temporary hook directory")
    }

    fn create_context(directory: &TempDir) -> PathBuf {
        let operation = OperationContext::new(
            Provider::Codex,
            ModelIdentity::known("fixture-model-blue".to_owned()).expect("valid fixture model"),
            "fixture-tool-use-blue".to_owned(),
        )
        .expect("create fixture operation context");
        context::create(&operation, directory.path()).expect("persist fixture context")
    }

    #[rstest]
    fn human_commit_message_is_unchanged_without_operation_context(hook_dir: TempDir) {
        let message_file = hook_dir.path().join("fixture-message.txt");
        let original = indoc! {"
            Fixture subject

            AI-Model: human-authored text
        "};
        std::fs::write(&message_file, original).expect("create fixture commit message");

        let result = super::run_prepare_commit_msg_with_context(
            &GitHookArgs {
                message_file: message_file.clone(),
            },
            None,
        );
        let actual = (
            result.is_ok(),
            std::fs::read_to_string(&message_file).expect("read fixture commit message"),
        );

        assert_eq!(actual, (true, original.to_owned()));
    }

    #[rstest]
    fn prepare_commit_msg_appends_provider_trailers(hook_dir: TempDir) {
        let message_file = hook_dir.path().join("fixture-message.txt");
        let original = indoc! {"Fixture subject"};
        let context_path = create_context(&hook_dir);
        std::fs::write(&message_file, original).expect("create fixture commit message");

        let result = super::run_prepare_commit_msg_with_context(
            &GitHookArgs {
                message_file: message_file.clone(),
            },
            Some(&context_path),
        );
        let actual = (
            result.is_ok(),
            std::fs::read_to_string(&message_file).expect("read fixture commit message"),
        );

        assert_eq!(
            actual,
            (
                true,
                indoc! {"
                    Fixture subject

                    Co-authored-by: Codex (fixture-model-blue) <noreply@openai.com>
                "}
                .to_owned(),
            ),
        );
    }

    #[rstest]
    fn partial_temporary_write_keeps_the_original_message(hook_dir: TempDir) {
        let message_file = hook_dir.path().join("fixture-message.txt");
        let original = indoc! {"Fixture subject"};
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
            result.is_err(),
            std::fs::read_to_string(&message_file).expect("read fixture commit message"),
        );

        assert_eq!(actual, (true, original.to_owned()));
    }

    #[derive(Clone, Copy)]
    enum ContextState {
        Missing,
        Broken,
        Expired,
    }

    #[rstest]
    #[case::missing_context(ContextState::Missing)]
    #[case::broken_context(ContextState::Broken)]
    #[case::expired_context(ContextState::Expired)]
    fn unusable_context_leaves_the_commit_message_unchanged(
        hook_dir: TempDir,
        #[case] context_state: ContextState,
    ) {
        let message_file = hook_dir.path().join("fixture-message.txt");
        let original = indoc! {"Fixture subject"};
        let context_path = match context_state {
            ContextState::Missing => hook_dir.path().join("fixture-missing-context.json"),
            ContextState::Broken => {
                let path = hook_dir.path().join("fixture-broken-context.json");
                std::fs::write(&path, "not-json").expect("create broken context");
                path
            }
            ContextState::Expired => {
                let path = create_context(&hook_dir);
                let mut value: Value =
                    serde_json::from_slice(&std::fs::read(&path).expect("read fixture context"))
                        .expect("parse fixture context");
                value["created_at"] = json!(context::unix_timestamp().expect("read clock") - 601);
                std::fs::write(
                    &path,
                    serde_json::to_vec(&value).expect("serialize expired context"),
                )
                .expect("expire fixture context");
                path
            }
        };
        std::fs::write(&message_file, original).expect("create fixture commit message");

        let result = super::run_prepare_commit_msg_with_context(
            &GitHookArgs {
                message_file: message_file.clone(),
            },
            Some(&context_path),
        );
        let actual = (
            result.is_ok(),
            std::fs::read_to_string(&message_file).expect("read fixture commit message"),
        );

        assert_eq!(actual, (true, original.to_owned()));
    }

    #[derive(clap::Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: AttributionCommands,
    }

    #[rstest]
    fn legacy_final_object_flag_is_accepted_without_enabling_validation(hook_dir: TempDir) {
        let parsed = TestCli::try_parse_from(["a", "pre-tool-use", "codex", "--final-object"])
            .expect("parse compatibility flag");
        let args = match parsed.command {
            AttributionCommands::PreToolUse(args) => args,
            _ => panic!("expected PreToolUse command"),
        };
        let raw = json!({
            "hook_input": {
                "hook_event_name": "PreToolUse",
                "tool_name": "Bash",
                "tool_use_id": "fixture-tool-use-final-object",
                "model": "fixture-model-blue",
                "tool_input": {"command": "git commit -m 'Fixture subject'"},
            },
            "hook_output": null,
        })
        .to_string();
        let output =
            process_pre_tool_use(&args, &raw, hook_dir.path()).expect("transformed output");
        let command = output
            .pointer("/hookSpecificOutput/updatedInput/command")
            .and_then(Value::as_str)
            .expect("transformed command");
        let context_path = command
            .strip_prefix(&format!("{CONTEXT_ENV}="))
            .and_then(|command| command.split_once(' '))
            .map(|(path, _)| path)
            .expect("context assignment");
        let normalized = output.to_string().replace(context_path, "<context-file>");

        assert_eq!(
            serde_json::from_str::<Value>(&normalized).expect("parse normalized output"),
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "allow",
                    "updatedInput": {
                        "command": format!("{CONTEXT_ENV}=<context-file> git commit -m 'Fixture subject'"),
                    },
                },
            }),
        );
    }

    #[rstest]
    fn legacy_unknown_reason_flag_does_not_change_proxy_output(hook_dir: TempDir) {
        let parsed = TestCli::try_parse_from([
            "a",
            "pre-tool-use",
            "codex",
            "--unknown-reason",
            "fixture model unavailable",
        ])
        .expect("parse compatibility option");
        let args = match parsed.command {
            AttributionCommands::PreToolUse(args) => args,
            _ => panic!("expected PreToolUse command"),
        };
        let proxy_output = json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow",
            },
        });
        let raw = json!({
            "hook_input": {
                "hook_event_name": "PreToolUse",
                "tool_name": "Bash",
                "tool_use_id": "fixture-tool-use-unknown-model",
                "tool_input": {"command": "git commit -m 'Fixture subject'"},
            },
            "hook_output": proxy_output,
        })
        .to_string();
        let output = process_pre_tool_use(&args, &raw, hook_dir.path());
        let context_count = std::fs::read_dir(hook_dir.path())
            .expect("read context directory")
            .count();

        assert_eq!((output, context_count), (Some(proxy_output), 0),);
    }

    #[rstest]
    fn malformed_envelope_preserves_existing_permission_output(hook_dir: TempDir) {
        let permission_output = json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "ask",
                "permissionDecisionReason": "fixture rule",
            },
        });
        let raw = json!({"hook_output": permission_output}).to_string();
        let args = PreToolUseArgs {
            provider: Provider::Codex,
            unknown_reason: None,
            final_object: false,
        };

        assert_eq!(
            process_pre_tool_use(&args, &raw, hook_dir.path()),
            Some(permission_output),
        );
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

        assert!(run(&parsed.command).is_ok());
    }
}
