use std::ffi::OsString;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use anyhow::{Context, Result};
use clap::Args;
use indoc::formatdoc;

use super::super::{bg_tasks, peer::notify, window_status};
use super::output;
use super::reminder::{self, ReminderSettings};
use crate::shared::command;
use crate::shared::log::short_run_id;
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
    let span = tracing::info_span!(
        "agent.bg_run",
        run_id = %short_run_id(),
        session = %args.session,
    );
    let _entered = span.enter();

    let reminder = ReminderSettings::load();
    let mut reminder_delivery = None;
    let status = bg_tasks::set_worker_pid(&args.session, &args.task_id, std::process::id())
        .and_then(|()| run_command(args, &reminder, &mut reminder_delivery));
    let execution_error = status.as_ref().err().map(|error| format!("{error:#}"));
    let exit_code = status.as_ref().ok().and_then(ExitStatus::code);

    bg_tasks::clear_best_effort(&args.session, &args.task_id);
    window_status::sync_window_status_for_session(&args.session);

    let message = completion_message(args, status.as_ref().ok(), execution_error.as_deref());
    send_completion_before_joining_reminder(
        || match notify::notify(&args.session, &message, None, None) {
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
                error = %format!("{error:#}"),
            ),
        },
        reminder_delivery,
        &args.session,
        &args.task_id,
    );

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

fn send_completion_before_joining_reminder(
    send_completion: impl FnOnce(),
    reminder_delivery: Option<JoinHandle<()>>,
    session: &str,
    task_id: &str,
) {
    send_completion();

    if let Some(reminder_delivery) = reminder_delivery
        && reminder_delivery.join().is_err()
    {
        tracing::warn!(
            event = "agent.bg_run.reminder_delivery_panicked",
            session = %session,
            task = %task_id,
        );
    }
}

fn run_command(
    args: &RunDetachedArgs,
    reminder: &ReminderSettings,
    reminder_delivery: &mut Option<JoinHandle<()>>,
) -> Result<ExitStatus> {
    let Some((program, command_args)) = args.command.split_first() else {
        anyhow::bail!("background command is empty");
    };
    let command_line = args
        .command
        .iter()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    let stdout = output::open_output_file(&args.stdout_file)
        .with_context(|| format!("failed to open stdout file: {}", args.stdout_file.display()))?;
    let stderr = output::open_output_file(&args.stderr_file)
        .with_context(|| format!("failed to open stderr file: {}", args.stderr_file.display()))?;

    let mut command = command::new(program);
    command
        .args(command_args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to start background command {program:?}"))?;
    if let Err(error) = bg_tasks::set_command_pid(&args.session, &args.task_id, child.id()) {
        tracing::warn!(
            event = "agent.bg_run.command_pid_record_failed",
            session = %args.session,
            task = %args.task_id,
            command_pid = child.id(),
            error = %error,
        );
    }
    let Some(timeout) = reminder.timeout_for(&command_line) else {
        return child
            .wait()
            .context("failed to wait for background command");
    };

    let started_at = Instant::now();
    reminder::wait_until_timeout_or_exit(
        timeout,
        || child.try_wait(),
        || started_at.elapsed(),
        reminder::sleep,
        || {
            let session = args.session.clone();
            let task_id = args.task_id.clone();
            let message = reminder_message(args, reminder.message());
            let span = tracing::Span::current();
            match thread::Builder::new()
                .name("agent-bg-reminder".to_string())
                .spawn(move || {
                    span.in_scope(|| match notify::notify(&session, &message, None, None) {
                        Ok(notify::Delivery::Queued { reason }) => tracing::warn!(
                            event = "agent.bg_run.reminder_queued",
                            session = %session,
                            task = %task_id,
                            reason,
                        ),
                        Ok(_) => tracing::info!(
                            event = "agent.bg_run.reminder_sent",
                            session = %session,
                            task = %task_id,
                        ),
                        Err(error) => tracing::warn!(
                            event = "agent.bg_run.reminder_failed",
                            session = %session,
                            task = %task_id,
                            error = %format!("{error:#}"),
                        ),
                    })
                }) {
                Ok(handle) => *reminder_delivery = Some(handle),
                Err(error) => tracing::warn!(
                    event = "agent.bg_run.reminder_thread_failed",
                    session = %args.session,
                    task = %args.task_id,
                    error = %error,
                ),
            }
        },
    )
    .context("failed to wait for background command")?
    .map_or_else(
        || {
            child
                .wait()
                .context("failed to wait for background command")
        },
        Ok,
    )
}

fn completion_message(
    args: &RunDetachedArgs,
    status: Option<&ExitStatus>,
    execution_error: Option<&str>,
) -> String {
    let task_id = strip_angle_brackets(&args.task_id);
    let command = strip_angle_brackets(&format!("{:?}", args.command));
    let exit = strip_angle_brackets(&exit_description(status, execution_error));
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

fn reminder_message(args: &RunDetachedArgs, message: &str) -> String {
    let task_id = strip_angle_brackets(&args.task_id);
    let command = strip_angle_brackets(&format!("{:?}", args.command));
    let stdout = strip_angle_brackets(&format!("{:?}", args.stdout_file));
    let stderr = strip_angle_brackets(&format!("{:?}", args.stderr_file));
    let message = strip_angle_brackets(message);

    formatdoc! {"
        <background-task-reminder>
        - Task ID: {task_id}
        - Command argv: {command}
        - stdout: {stdout}
        - stderr: {stderr}

        {message}
        </background-task-reminder>"
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

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn reminder_message_identifies_the_background_task() {
        let args = RunDetachedArgs {
            session: "fixture-session".to_string(),
            task_id: "fixture-task".to_string(),
            stdout_file: PathBuf::from("/tmp/fixture/stdout.log"),
            stderr_file: PathBuf::from("/tmp/fixture/stderr.log"),
            command: vec![OsString::from("sample-command"), OsString::from("--wait")],
        };

        assert_eq!(
            reminder_message(&args, "Reminder text"),
            indoc! {r#"
                <background-task-reminder>
                - Task ID: fixture-task
                - Command argv: ["sample-command", "--wait"]
                - stdout: "/tmp/fixture/stdout.log"
                - stderr: "/tmp/fixture/stderr.log"

                Reminder text
                </background-task-reminder>"#},
        );
    }

    #[test]
    fn completion_notification_does_not_wait_for_reminder_delivery() {
        let (reminder_started_tx, reminder_started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let reminder_delivery = thread::Builder::new()
            .name("fixture-reminder".to_string())
            .spawn(move || {
                reminder_started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            })
            .unwrap();
        let (completion_sent_tx, completion_sent_rx) = mpsc::channel();
        let completion = thread::spawn(move || {
            send_completion_before_joining_reminder(
                || completion_sent_tx.send(()).unwrap(),
                Some(reminder_delivery),
                "fixture-session",
                "fixture-task",
            );
        });

        let reminder_started = reminder_started_rx
            .recv_timeout(Duration::from_secs(5))
            .is_ok();
        let completion_sent = completion_sent_rx
            .recv_timeout(Duration::from_secs(5))
            .is_ok();
        release_tx.send(()).unwrap();
        completion.join().unwrap();

        assert_eq!((reminder_started, completion_sent), (true, true));
    }
}
