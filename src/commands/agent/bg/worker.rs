use std::ffi::OsString;
use std::fs::File;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};

use anyhow::{Context, Result};
use clap::Args;
use indoc::formatdoc;

use super::super::{bg_tasks, peer::notify};
use crate::shared::command;
use crate::shared::sanitize::strip_angle_brackets;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct RunDetachedArgs {
    /// Session that should receive the completion notification.
    #[arg(long)]
    session: String,

    /// Background task identifier used by the session task registry.
    #[arg(long)]
    task_id: String,

    /// File that receives the command's stdout.
    #[arg(long, value_name = "FILE")]
    stdout_file: PathBuf,

    /// File that receives the command's stderr.
    #[arg(long, value_name = "FILE")]
    stderr_file: PathBuf,

    /// Command and arguments to execute.
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    command: Vec<OsString>,
}

pub fn run(args: &RunDetachedArgs) -> Result<()> {
    let status = run_command(args);
    let execution_error = status.as_ref().err().map(ToString::to_string);
    let exit_code = status.as_ref().ok().and_then(ExitStatus::code);

    if let Err(error) = bg_tasks::clear(&args.session, &args.task_id) {
        tracing::warn!(
            event = "agent.bg_run.task_clear_failed",
            session = %args.session,
            task = %args.task_id,
            error = %error,
        );
    }

    let message = completion_message(args, status.as_ref().ok(), execution_error.as_deref());
    match notify::notify(&args.session, &message, None, None) {
        Ok(notify::Delivery::Queued { reason }) => tracing::warn!(
            event = "agent.bg_run.notification_queued",
            session = %args.session,
            task = %args.task_id,
            reason,
        ),
        Ok(_) => tracing::info!(
            event = "agent.bg_run.completed",
            session = %args.session,
            task = %args.task_id,
            exit_code = ?exit_code,
        ),
        Err(error) => tracing::warn!(
            event = "agent.bg_run.notification_failed",
            session = %args.session,
            task = %args.task_id,
            error = %error,
        ),
    }

    if let Some(error) = execution_error {
        tracing::warn!(
            event = "agent.bg_run.command_failed",
            session = %args.session,
            task = %args.task_id,
            error,
        );
    }
    Ok(())
}

fn run_command(args: &RunDetachedArgs) -> Result<ExitStatus> {
    let Some((program, command_args)) = args.command.split_first() else {
        anyhow::bail!("background command is empty");
    };
    let stdout = File::create(&args.stdout_file)
        .with_context(|| format!("failed to open stdout file: {}", args.stdout_file.display()))?;
    let stderr = File::create(&args.stderr_file)
        .with_context(|| format!("failed to open stderr file: {}", args.stderr_file.display()))?;

    command::new(program)
        .args(command_args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .status()
        .with_context(|| format!("failed to start background command {program:?}"))
}

fn completion_message(
    args: &RunDetachedArgs,
    status: Option<&ExitStatus>,
    execution_error: Option<&str>,
) -> String {
    let task_id = strip_angle_brackets(&args.task_id);
    let command = strip_angle_brackets(&format!("{:?}", args.command));
    let exit = exit_description(status, execution_error);
    let stdout = strip_angle_brackets(&format!("{:?}", args.stdout_file));
    let stderr = strip_angle_brackets(&format!("{:?}", args.stderr_file));

    formatdoc! {"
        <background-task-complete>
        - Task ID: {task_id}
        - Command argv: {command}
        - Exit code: {exit}
        - stdout: {stdout}
        - stderr: {stderr}
        </background-task-complete>"
    }
}

fn exit_description(status: Option<&ExitStatus>, execution_error: Option<&str>) -> String {
    if let Some(code) = status.and_then(ExitStatus::code) {
        return code.to_string();
    }
    if let Some(signal) = status.and_then(ExitStatusExt::signal) {
        return format!("unavailable (terminated by signal {signal})");
    }
    if let Some(error) = execution_error {
        return format!("unavailable (failed to run command: {:?})", error);
    }
    "unavailable".to_string()
}
