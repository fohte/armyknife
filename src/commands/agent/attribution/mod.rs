mod command;
mod context;
mod hook;
mod message;
mod transcript;

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use self::context::ModelIdentity;

const CONTEXT_ENV: &str = "ARMYKNIFE_AGENT_ATTRIBUTION_CONTEXT";
const REQUIRED_ENV: &str = "ARMYKNIFE_AGENT_ATTRIBUTION_REQUIRED";

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
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct PreToolUseArgs {
    /// Provider whose original PreToolUse JSON is on stdin
    #[arg(value_enum)]
    provider: Provider,

    /// Explicitly record an unknown model when runtime evidence is unavailable
    #[arg(long)]
    unknown_reason: Option<String>,
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct GitHookArgs {
    /// Commit message file supplied by Git
    message_file: PathBuf,
}

pub fn run(command: &AttributionCommands) -> Result<()> {
    match command {
        AttributionCommands::PreToolUse(args) => run_pre_tool_use(args),
        AttributionCommands::PrepareCommitMsg(args) => run_prepare_commit_msg(args),
        AttributionCommands::CommitMsg(args) => run_commit_msg(args),
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

    let result = hook::transform(
        args.provider,
        &stdin,
        args.unknown_reason.as_deref(),
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
    let Some(context_path) = required_context_path()? else {
        return Ok(());
    };
    let message_text = std::fs::read_to_string(&args.message_file).with_context(|| {
        format!(
            "failed to read commit message file {}",
            args.message_file.display()
        )
    })?;
    let updated = prepare_message_with_context(Some(&context_path), &message_text)?;
    std::fs::write(&args.message_file, updated).with_context(|| {
        format!(
            "failed to update commit message file {}",
            args.message_file.display()
        )
    })
}

fn run_commit_msg(args: &GitHookArgs) -> Result<()> {
    let context_path = required_context_path()?;
    let Some(context_path) = context_path else {
        return Ok(());
    };
    let message_text = std::fs::read_to_string(&args.message_file).with_context(|| {
        format!(
            "failed to read commit message file {}",
            args.message_file.display()
        )
    })?;
    verify_message_with_context(Some(&context_path), &message_text)
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

fn required_context_path() -> Result<Option<PathBuf>> {
    match (
        std::env::var_os(REQUIRED_ENV),
        std::env::var_os(CONTEXT_ENV),
    ) {
        (None, None) => Ok(None),
        (Some(required), Some(path)) if required == "1" => Ok(Some(PathBuf::from(path))),
        (Some(_), Some(_)) => anyhow::bail!("invalid attribution-required marker"),
        _ => anyhow::bail!("attribution context environment is incomplete"),
    }
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
    use super::{
        CONTEXT_ENV, GitHookArgs, Provider, REQUIRED_ENV, prepare_message_with_context,
        run_prepare_commit_msg, verify_message_with_context,
    };
    use crate::commands::agent::attribution::context::{self, ModelIdentity, OperationContext};
    use indoc::indoc;
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    #[fixture]
    fn hook_dir() -> TempDir {
        tempfile::tempdir().expect("create temporary hook directory")
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

        let actual = temp_env::with_vars(
            [(REQUIRED_ENV, required_value), (CONTEXT_ENV, context_value)],
            || {
                let result = run_prepare_commit_msg(&args).map_err(|error| error.to_string());
                let message =
                    std::fs::read_to_string(&message_file).expect("read fixture commit message");
                (result, message)
            },
        );

        assert_eq!(actual, (expected_result, expected_message));
    }
}
