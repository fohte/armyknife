mod command;
mod context;
mod hook;
mod message;
mod origin;
mod reference_transaction;
mod transcript;

use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use self::context::ModelIdentity;

const CONTEXT_ENV: &str = "ARMYKNIFE_AGENT_ATTRIBUTION_CONTEXT";
const REQUIRED_ENV: &str = "ARMYKNIFE_AGENT_ATTRIBUTION_REQUIRED";
const FINAL_OBJECT_REQUIRED_ENV: &str = "ARMYKNIFE_AGENT_ATTRIBUTION_FINAL_OBJECT_REQUIRED";
const PROVIDER_ORIGIN_ENV: &str = "ARMYKNIFE_AGENT_ATTRIBUTION_PROVIDER_ORIGIN";
const PROVIDER_ORIGIN_ENV_VARS: [&str; 4] = [
    "CLAUDE_CODE_SESSION_ID",
    "CODEX_SESSION_ID",
    "CODEX_THREAD_ID",
    PROVIDER_ORIGIN_ENV,
];

#[derive(Debug, Clone, Copy, Deserialize, Serialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum Provider {
    ClaudeCode,
    Codex,
}

impl Provider {
    fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
        }
    }
}

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum AttributionCommands {
    /// Transform one provider PreToolUse request after the existing permission proxy
    #[command(name = "pre-tool-use")]
    PreToolUse(PreToolUseArgs),

    /// Add machine-managed trailers to a commit message
    #[command(name = "prepare-commit-msg", hide = true)]
    PrepareCommitMsg(GitHookArgs),

    /// Check the machine-managed trailers before a commit
    #[command(name = "commit-msg", hide = true)]
    CommitMsg(GitHookArgs),

    /// Verify the final commit object before a reference is updated
    #[command(name = "reference-transaction", hide = true)]
    ReferenceTransaction(ReferenceTransactionArgs),
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct PreToolUseArgs {
    /// Provider whose original PreToolUse JSON is on stdin
    #[arg(value_enum)]
    provider: Provider,

    /// Explicitly record an unknown model when runtime evidence is unavailable
    #[arg(long)]
    unknown_reason: Option<String>,

    /// Verify the final commit object during Git's reference transaction
    #[arg(long)]
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
        eprintln!("failed to read attribution hook input: {error}");
        print_json(&hook::blocked_output(
            args.provider,
            "Attribution context is unavailable for this commit.",
        ))?;
        return Ok(());
    }

    let result = hook::transform_with_final_object(
        args.provider,
        &stdin,
        args.unknown_reason.as_deref(),
        args.final_object,
        &context::default_context_dir(),
    );
    match result {
        Ok(Some(output)) => print_json(&output),
        Ok(None) => Ok(()),
        Err(error) => {
            eprintln!("agent commit attribution blocked: {error:#}");
            print_json(&hook::blocked_output(
                args.provider,
                "Attribution context is unavailable for this commit.",
            ))
        }
    }
}

fn print_json(value: &Value) -> Result<()> {
    serde_json::to_writer(std::io::stdout(), value).context("failed to write hook output")?;
    println!();
    Ok(())
}

fn run_prepare_commit_msg(args: &GitHookArgs) -> Result<()> {
    run_prepare_commit_msg_with_final_object_environment(
        args,
        std::env::var_os(REQUIRED_ENV),
        std::env::var_os(CONTEXT_ENV),
        provider_origin_is_active(std::env::vars_os()),
        std::env::var_os(FINAL_OBJECT_REQUIRED_ENV),
    )
}

#[cfg(test)]
fn run_prepare_commit_msg_with_environment(
    args: &GitHookArgs,
    required_marker: Option<OsString>,
    context_path_value: Option<OsString>,
    provider_session: bool,
) -> Result<()> {
    run_prepare_commit_msg_with_final_object_environment(
        args,
        required_marker,
        context_path_value,
        provider_session,
        None,
    )
}

fn run_prepare_commit_msg_with_final_object_environment(
    args: &GitHookArgs,
    required_marker: Option<OsString>,
    context_path_value: Option<OsString>,
    provider_session: bool,
    final_object_marker: Option<OsString>,
) -> Result<()> {
    let final_object = final_object_required(final_object_marker)?;
    let Some(context_path) = required_context_path_from_environment(
        required_marker,
        context_path_value,
        provider_session,
    )?
    else {
        return Ok(());
    };
    let message_text = std::fs::read_to_string(&args.message_file).with_context(|| {
        format!(
            "failed to read commit message file {}",
            args.message_file.display()
        )
    })?;
    let updated = if final_object {
        message::prepare_for_final_object(
            &context_path,
            &message_text,
            reference_transaction::capture_target()?,
        )?
    } else {
        prepare_message_with_context(Some(&context_path), &message_text)?
    };
    std::fs::write(&args.message_file, updated).with_context(|| {
        format!(
            "failed to update commit message file {}",
            args.message_file.display()
        )
    })
}

fn run_commit_msg(args: &GitHookArgs) -> Result<()> {
    run_commit_msg_with_final_object_environment(
        args,
        std::env::var_os(REQUIRED_ENV),
        std::env::var_os(CONTEXT_ENV),
        provider_origin_is_active(std::env::vars_os()),
        std::env::var_os(FINAL_OBJECT_REQUIRED_ENV),
    )
}

#[cfg(test)]
fn run_commit_msg_with_environment(
    args: &GitHookArgs,
    required_marker: Option<OsString>,
    context_path_value: Option<OsString>,
    provider_session: bool,
) -> Result<()> {
    run_commit_msg_with_final_object_environment(
        args,
        required_marker,
        context_path_value,
        provider_session,
        None,
    )
}

fn run_commit_msg_with_final_object_environment(
    args: &GitHookArgs,
    required_marker: Option<OsString>,
    context_path_value: Option<OsString>,
    provider_session: bool,
    final_object_marker: Option<OsString>,
) -> Result<()> {
    let final_object = final_object_required(final_object_marker)?;
    let context_path = required_context_path_from_environment(
        required_marker,
        context_path_value,
        provider_session,
    )?;
    let Some(context_path) = context_path else {
        return Ok(());
    };
    let message_text = std::fs::read_to_string(&args.message_file).with_context(|| {
        format!(
            "failed to read commit message file {}",
            args.message_file.display()
        )
    })?;
    if final_object {
        message::verify_without_consuming(&context_path, &message_text)
    } else {
        verify_message_with_context(Some(&context_path), &message_text)
    }
}

fn run_reference_transaction(args: &ReferenceTransactionArgs) -> Result<()> {
    if !final_object_required(std::env::var_os(FINAL_OBJECT_REQUIRED_ENV))? {
        return Ok(());
    }
    if args.state != "prepared" {
        return reference_transaction::run(&args.state, "", None);
    }
    let context_path = std::env::var_os(CONTEXT_ENV)
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("final-object attribution context path is missing"))?;
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .context("failed to read reference-transaction updates")?;
    reference_transaction::run(&args.state, &input, Some(&context_path))
}

fn final_object_required(value: Option<OsString>) -> Result<bool> {
    match value {
        None => Ok(false),
        Some(value) if value == OsStr::new("1") => Ok(true),
        Some(_) => anyhow::bail!("invalid final-object attribution marker"),
    }
}

fn prepare_message_with_context(context_path: Option<&Path>, message_text: &str) -> Result<String> {
    match context_path {
        Some(path) => message::prepare(path, message_text),
        None => Ok(message_text.to_owned()),
    }
}

fn verify_message_with_context(context_path: Option<&Path>, message_text: &str) -> Result<()> {
    match context_path {
        Some(path) => message::verify_and_consume(path, message_text),
        None => Ok(()),
    }
}

fn required_context_path_from_environment(
    required_marker: Option<OsString>,
    context_path: Option<OsString>,
    provider_session: bool,
) -> Result<Option<PathBuf>> {
    match (required_marker, context_path) {
        (None, None) if provider_session => {
            anyhow::bail!("AI provider session has no operation attribution context")
        }
        (None, None) => Ok(None),
        (Some(required), Some(path)) if required.as_os_str() == OsStr::new("1") => {
            Ok(Some(PathBuf::from(path)))
        }
        (Some(_), Some(_)) => anyhow::bail!("invalid attribution-required marker"),
        _ => anyhow::bail!("attribution context environment is incomplete"),
    }
}

fn provider_origin_is_active(variables: impl IntoIterator<Item = (OsString, OsString)>) -> bool {
    variables.into_iter().any(|(name, value)| {
        !value.is_empty()
            && PROVIDER_ORIGIN_ENV_VARS
                .iter()
                .any(|marker| name == OsStr::new(marker))
    })
}

fn model_identity(model: Option<String>, unknown_reason: Option<&str>) -> Result<ModelIdentity> {
    match model {
        Some(slug) => match ModelIdentity::known(slug) {
            Ok(model) => Ok(model),
            Err(_) => ModelIdentity::unknown(unknown_reason),
        },
        _ => ModelIdentity::unknown(unknown_reason),
    }
}

#[derive(Debug, Deserialize)]
struct ProxyEnvelope {
    hook_input: Value,
    #[serde(default)]
    hook_output: Option<Value>,
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{
        GitHookArgs, PROVIDER_ORIGIN_ENV, Provider, hook, prepare_message_with_context,
        provider_origin_is_active, run_commit_msg_with_environment,
        run_commit_msg_with_final_object_environment, run_prepare_commit_msg_with_environment,
        verify_message_with_context,
    };
    use crate::commands::agent::attribution::context::{self, ModelIdentity, OperationContext};
    use crate::commands::agent::attribution::message;
    use indoc::indoc;
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    #[fixture]
    fn hook_dir() -> TempDir {
        tempfile::tempdir().expect("create temporary hook directory")
    }

    #[fixture]
    fn codex_provider_session() -> bool {
        provider_origin_is_active([
            (
                OsString::from("CODEX_SESSION_ID"),
                OsString::from("fixture-session-id"),
            ),
            (
                OsString::from("CODEX_THREAD_ID"),
                OsString::from("fixture-thread-id"),
            ),
        ])
    }

    #[test]
    fn human_commit_message_is_unchanged_without_operation_context() {
        let original = indoc! {"
            Fixture subject

            AI-Model: human-authored text
        "};
        let prepared =
            prepare_message_with_context(None, original).map_err(|error| error.to_string());
        let verified =
            verify_message_with_context(None, original).map_err(|error| error.to_string());

        assert_eq!((prepared, verified), (Ok(original.to_owned()), Ok(())),);
    }

    #[rstest]
    #[case::human(None, false, false, None, false)]
    #[case::missing_context(
        Some("1"),
        false,
        false,
        Some("attribution context environment is incomplete"),
        false
    )]
    #[case::missing_required_marker(
        None,
        true,
        false,
        Some("attribution context environment is incomplete"),
        false
    )]
    #[case::invalid_marker(
        Some("invalid"),
        true,
        false,
        Some("invalid attribution-required marker"),
        false
    )]
    #[case::valid_context(Some("1"), true, true, None, true)]
    fn prepare_commit_msg_checks_the_attribution_environment(
        #[case] required_value: Option<&str>,
        #[case] with_context: bool,
        #[case] create_context: bool,
        #[case] expected_error: Option<&str>,
        #[case] expect_trailers: bool,
        hook_dir: TempDir,
    ) {
        let message_file = hook_dir.path().join("fixture-message.txt");
        let original = indoc! {"Fixture subject"};
        std::fs::write(&message_file, original).expect("create fixture commit message");
        let context_path = if create_context {
            let operation = OperationContext::new(
                Provider::Codex,
                ModelIdentity::known("fixture-model-red".to_owned()).expect("valid fixture model"),
                "fixture-tool-use-red".to_owned(),
            )
            .expect("create fixture operation context");
            context::create(&operation, hook_dir.path()).expect("persist fixture context")
        } else {
            hook_dir.path().join("fixture-context.json")
        };
        let context_value =
            with_context.then(|| context_path.to_str().expect("context path is UTF-8"));
        let expected_result = expected_error.map_or(Ok(()), |error| Err(error.to_owned()));
        let expected_message = if expect_trailers {
            indoc! {"
                Fixture subject

                AI-Tool: Codex
                AI-Model: fixture-model-red
            "}
            .to_owned()
        } else {
            original.to_owned()
        };
        let args = GitHookArgs {
            message_file: message_file.clone(),
        };

        let result = run_prepare_commit_msg_with_environment(
            &args,
            required_value.map(OsString::from),
            context_value.map(OsString::from),
            false,
        )
        .map_err(|error| error.to_string());
        let message = std::fs::read_to_string(&message_file).expect("read fixture commit message");
        let actual = (result, message);

        assert_eq!(actual, (expected_result, expected_message));
    }

    #[rstest]
    #[case::claude_session("CLAUDE_CODE_SESSION_ID")]
    #[case::codex_session("CODEX_SESSION_ID")]
    #[case::codex_thread("CODEX_THREAD_ID")]
    fn provider_commit_without_operation_context_is_rejected(
        #[case] marker: &str,
        hook_dir: TempDir,
    ) {
        let message_file = hook_dir.path().join("fixture-message.txt");
        let original = indoc! {"Fixture subject"};
        std::fs::write(&message_file, original).expect("create fixture commit message");
        let args = GitHookArgs {
            message_file: message_file.clone(),
        };
        let provider_session = provider_origin_is_active([(
            OsString::from(marker),
            OsString::from("fixture-session-id"),
        )]);
        let result = run_prepare_commit_msg_with_environment(&args, None, None, provider_session)
            .map_err(|error| error.to_string());
        let message = std::fs::read_to_string(&message_file).expect("read fixture commit message");

        assert_eq!(
            (result, message),
            (
                Err("AI provider session has no operation attribution context".to_owned()),
                original.to_owned(),
            ),
        );
    }

    #[rstest]
    #[case::empty_value("CODEX_THREAD_ID", "")]
    #[case::unrelated_variable("CLAUDECODE", "1")]
    fn non_session_environment_does_not_mark_an_ai_origin(#[case] name: &str, #[case] value: &str) {
        assert!(!provider_origin_is_active([(
            OsString::from(name),
            OsString::from(value),
        )]));
    }

    #[rstest]
    fn human_commit_without_provider_context_remains_unchanged(hook_dir: TempDir) {
        let message_file = hook_dir.path().join("fixture-message.txt");
        let original = indoc! {"
            Fixture subject

            AI-Model: human-authored text
        "};
        std::fs::write(&message_file, original).expect("create fixture commit message");
        let args = GitHookArgs {
            message_file: message_file.clone(),
        };
        let result = run_prepare_commit_msg_with_environment(&args, None, None, false)
            .map_err(|error| error.to_string());
        let message = std::fs::read_to_string(&message_file).expect("read fixture commit message");

        assert_eq!((result, message), (Ok(()), original.to_owned()));
    }

    #[rstest]
    fn provider_commit_without_operation_context_is_rejected_by_commit_msg_hook(
        codex_provider_session: bool,
        hook_dir: TempDir,
    ) {
        let message_file = hook_dir.path().join("fixture-message.txt");
        let original = indoc! {"Fixture subject"};
        std::fs::write(&message_file, original).expect("create fixture commit message");
        let args = GitHookArgs {
            message_file: message_file.clone(),
        };
        let result = run_commit_msg_with_environment(&args, None, None, codex_provider_session)
            .map_err(|error| error.to_string());
        let message = std::fs::read_to_string(&message_file).expect("read fixture commit message");

        assert_eq!(
            (result, message),
            (
                Err("AI provider session has no operation attribution context".to_owned()),
                original.to_owned(),
            ),
        );
    }

    #[rstest]
    fn final_object_commit_msg_hook_checks_without_consuming_context(hook_dir: TempDir) {
        let operation = OperationContext::new(
            Provider::Codex,
            ModelIdentity::known("fixture-model-red".to_owned()).expect("valid fixture model"),
            "fixture-tool-use-red".to_owned(),
        )
        .expect("create fixture operation context");
        let context_path = context::create(&operation, hook_dir.path())
            .expect("persist fixture operation context");
        let message = message::prepare_for_final_object(
            &context_path,
            "Fixture subject",
            context::CommitTarget {
                repository: "/fixture/repository".to_owned(),
                reference: "refs/heads/fixture-branch".to_owned(),
                old_oid: Some("1111111111111111111111111111111111111111".to_owned()),
            },
        )
        .expect("prepare final-object message");
        let message_file = hook_dir.path().join("fixture-message.txt");
        std::fs::write(&message_file, message).expect("create fixture commit message");
        let args = GitHookArgs { message_file };
        let result = run_commit_msg_with_final_object_environment(
            &args,
            Some(OsString::from("1")),
            Some(OsString::from(
                context_path.to_str().expect("context path is UTF-8"),
            )),
            false,
            Some(OsString::from("1")),
        )
        .map_err(|error| error.to_string());
        let claim_remains = context::claimed_final_object_context(&context_path).is_ok();

        assert_eq!((result, claim_remains), (Ok(()), true));
    }

    #[rstest]
    fn unrecognized_codex_git_alias_commit_is_rejected_by_git_hook(hook_dir: TempDir) {
        let raw_input = serde_json::json!({
            "hook_input": {
                "hook_event_name": "PreToolUse",
                "tool_name": "Bash",
                "tool_use_id": "fixture-tool-use-alias",
                "model": "fixture-model-red-2026",
                "tool_input": {
                    "command": "env -u CODEX_SESSION_ID -u CODEX_THREAD_ID git ci -m 'Fixture subject'"
                },
            },
            "hook_output": null,
        })
        .to_string();
        let pre_tool_use = hook::transform(Provider::Codex, &raw_input, None, hook_dir.path())
            .map_err(|error| error.to_string());
        let message_file = hook_dir.path().join("fixture-message.txt");
        let original = indoc! {"Fixture subject"};
        std::fs::write(&message_file, original).expect("create fixture commit message");
        let args = GitHookArgs {
            message_file: message_file.clone(),
        };
        let origin_marker =
            provider_origin_is_active([(OsString::from(PROVIDER_ORIGIN_ENV), OsString::from("1"))]);
        let git_hook = run_prepare_commit_msg_with_environment(&args, None, None, origin_marker)
            .map_err(|error| error.to_string());
        let message = std::fs::read_to_string(&message_file).expect("read fixture commit message");
        let context_file_count = std::fs::read_dir(hook_dir.path())
            .expect("read context directory")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "json")
            })
            .count();

        assert_eq!(
            (pre_tool_use, git_hook, message, context_file_count,),
            (
                Ok(Some(serde_json::json!({
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": "allow",
                        "updatedInput": {
                            "command": format!(
                                "env -u CODEX_SESSION_ID -u CODEX_THREAD_ID '{PROVIDER_ORIGIN_ENV}=1' git ci -m 'Fixture subject'"
                            ),
                        },
                    },
                }))),
                Err("AI provider session has no operation attribution context".to_owned()),
                original.to_owned(),
                0,
            ),
        );
    }

    #[rstest]
    fn persistent_codex_shell_commit_via_write_stdin_is_rejected_without_fresh_context(
        hook_dir: TempDir,
    ) {
        let raw_input = serde_json::json!({
            "hook_input": {
                "hook_event_name": "PreToolUse",
                "tool_name": "Bash",
                "tool_use_id": "fixture-tool-use-shell",
                "model": "fixture-model-red-2026",
                "tool_input": {"command": "bash -i"},
            },
            "hook_output": null,
        })
        .to_string();
        let shell_start = hook::transform(Provider::Codex, &raw_input, None, hook_dir.path())
            .map_err(|error| error.to_string());
        let message_file = hook_dir.path().join("fixture-message.txt");
        let original = indoc! {"Fixture subject"};
        std::fs::write(&message_file, original).expect("create fixture commit message");
        let args = GitHookArgs {
            message_file: message_file.clone(),
        };
        let origin_marker =
            provider_origin_is_active([(OsString::from(PROVIDER_ORIGIN_ENV), OsString::from("1"))]);
        let git_hook = run_prepare_commit_msg_with_environment(&args, None, None, origin_marker)
            .map_err(|error| error.to_string());
        let message = std::fs::read_to_string(&message_file).expect("read fixture commit message");

        assert_eq!(
            (shell_start, git_hook, message),
            (
                Ok(Some(serde_json::json!({
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": "allow",
                        "updatedInput": {
                            "command": format!("{PROVIDER_ORIGIN_ENV}=1 bash -i"),
                        },
                    },
                }))),
                Err("AI provider session has no operation attribution context".to_owned()),
                original.to_owned(),
            ),
        );
    }
}
