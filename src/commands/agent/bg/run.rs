use std::ffi::OsString;
use std::fs::{self, File};
use std::path::Path;

use anyhow::{Context, Result, bail};
use clap::Args;
use uuid::Uuid;

use super::super::{bg_tasks, store, types::SessionStatus};
use crate::infra::process;
use crate::shared::cache;
use crate::shared::env_var::EnvVars;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct RunArgs {
    /// Command and arguments to run after `--`.
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    pub command: Vec<OsString>,
}

pub fn run(args: &RunArgs) -> Result<()> {
    let session_id = EnvVars::load()
        .own_session_id()
        .context("a agent bg run must be called from a tracked Claude Code or Codex session")?;
    let session = store::load_session(&session_id)?
        .with_context(|| format!("session {session_id} is not tracked by armyknife"))?;
    if session.status == SessionStatus::Ended {
        bail!("session {session_id} has ended");
    }

    let executable =
        std::env::current_exe().context("failed to resolve the armyknife executable")?;
    let task_id = Uuid::new_v4().to_string();
    let output_dir = cache::base_dir()
        .context("Unable to determine cache directory")?
        .join("agent-bg")
        .join("output");
    fs::create_dir_all(&output_dir).with_context(|| {
        format!(
            "failed to create background command output directory: {}",
            output_dir.display()
        )
    })?;
    let stdout_path = output_dir.join(format!("{task_id}.stdout"));
    let stderr_path = output_dir.join(format!("{task_id}.stderr"));
    File::create(&stdout_path)
        .with_context(|| format!("failed to create stdout file: {}", stdout_path.display()))?;
    if let Err(error) = File::create(&stderr_path) {
        let _ = fs::remove_file(&stdout_path);
        return Err(error)
            .with_context(|| format!("failed to create stderr file: {}", stderr_path.display()));
    }

    if let Err(error) = bg_tasks::register(&session_id, &task_id) {
        let _ = fs::remove_file(&stdout_path);
        let _ = fs::remove_file(&stderr_path);
        return Err(error);
    }
    let worker_args = worker_args(
        &session_id,
        &task_id,
        &stdout_path,
        &stderr_path,
        &args.command,
    );
    if let Err(error) = process::spawn_detached(executable, worker_args, None, &[]) {
        clear_after_spawn_failure(&session_id, &task_id, &stdout_path, &stderr_path);
        return Err(error).context("failed to start the detached background command worker");
    }

    println!("Started background task {task_id}");
    println!("stdout: {}", stdout_path.display());
    println!("stderr: {}", stderr_path.display());
    Ok(())
}

fn worker_args(
    session_id: &str,
    task_id: &str,
    stdout_path: &Path,
    stderr_path: &Path,
    command: &[OsString],
) -> Vec<OsString> {
    let mut args = ["agent", "bg", "run-detached"]
        .into_iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
    args.extend([
        "--session".into(),
        session_id.into(),
        "--task-id".into(),
        task_id.into(),
        "--stdout-file".into(),
        stdout_path.as_os_str().to_os_string(),
        "--stderr-file".into(),
        stderr_path.as_os_str().to_os_string(),
        "--".into(),
    ]);
    args.extend(command.iter().cloned());
    args
}

fn clear_after_spawn_failure(
    session_id: &str,
    task_id: &str,
    stdout_path: &Path,
    stderr_path: &Path,
) {
    if let Err(error) = bg_tasks::clear(session_id, task_id) {
        tracing::warn!(
            event = "agent.bg_run.task_clear_failed",
            session = session_id,
            task = task_id,
            error = %error,
        );
    }
    let _ = fs::remove_file(stdout_path);
    let _ = fs::remove_file(stderr_path);
}
