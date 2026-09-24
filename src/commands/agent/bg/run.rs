use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Args;
use uuid::Uuid;

use super::super::{bg_tasks, store, types::SessionStatus, window_status};
use crate::infra::process;
use crate::shared::env_var::EnvVars;

use super::output;

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
    let output_dir = output::output_dir(&task_id)?;
    let stdout_path = output_dir.join(format!("{task_id}.stdout"));
    let stderr_path = output_dir.join(format!("{task_id}.stderr"));
    let mut guard = BackgroundRunGuard::new(&session_id, &task_id);
    guard.track_output_dir(output_dir);
    output::create_output_file(&stdout_path)
        .with_context(|| format!("failed to create stdout file: {}", stdout_path.display()))?;
    guard.track_output(stdout_path.clone());
    output::create_output_file(&stderr_path)
        .with_context(|| format!("failed to create stderr file: {}", stderr_path.display()))?;
    guard.track_output(stderr_path.clone());

    bg_tasks::register(&session_id, &task_id)?;
    guard.mark_task_registered();
    window_status::sync_window_status_for_session(&session_id);
    let worker_args = worker_args(
        &session_id,
        &task_id,
        &stdout_path,
        &stderr_path,
        &args.command,
    );
    let worker_pid = process::spawn_detached_with_pid(executable, worker_args, None, &[])
        .context("failed to start the detached background command worker")?;
    guard.disarm();
    if let Err(error) = bg_tasks::set_worker_pid(&session_id, &task_id, worker_pid) {
        tracing::warn!(
            event = "agent.bg_run.worker_pid_record_failed",
            session = %session_id,
            task = %task_id,
            worker_pid,
            error = %error,
        );
    }

    println!("Started background task {task_id}");
    println!("stdout: {}", stdout_path.display());
    println!("stderr: {}", stderr_path.display());
    Ok(())
}

struct BackgroundRunGuard {
    session_id: String,
    task_id: String,
    output_dir: Option<PathBuf>,
    output_paths: Vec<PathBuf>,
    task_registered: bool,
    armed: bool,
}

impl BackgroundRunGuard {
    fn new(session_id: &str, task_id: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            task_id: task_id.to_string(),
            output_dir: None,
            output_paths: Vec::new(),
            task_registered: false,
            armed: true,
        }
    }

    fn track_output(&mut self, path: PathBuf) {
        self.output_paths.push(path);
    }

    fn track_output_dir(&mut self, dir: PathBuf) {
        self.output_dir = Some(dir);
    }

    fn mark_task_registered(&mut self) {
        self.task_registered = true;
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for BackgroundRunGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if self.task_registered {
            bg_tasks::clear_best_effort(&self.session_id, &self.task_id);
            window_status::sync_window_status_for_session(&self.session_id);
        }
        for path in &self.output_paths {
            let _ = fs::remove_file(path);
        }
        if let Some(dir) = &self.output_dir {
            let _ = fs::remove_dir(dir);
        }
    }
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
