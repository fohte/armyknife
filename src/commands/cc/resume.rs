use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, bail};
use clap::Args;

use super::error::CcError;
use super::types::TMUX_SESSION_OPTION;
use crate::infra::{process, tmux};
use crate::shared::cache;
use crate::shared::command::find_command_path;

const LOCK_RETRY_DELAY: Duration = Duration::from_millis(100);

const LOCK_TIMEOUT: Duration = Duration::from_secs(300);

/// Minimum spacing between two consecutive `claude --resume` startups.
/// Must be long enough for the first claude to finish reading its
/// session-id files before the next one starts.
const LOCK_HOLD: Duration = Duration::from_millis(500);

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ResumeArgs {
    /// Claude Code session ID to resume. When omitted, the session ID is read from
    /// the current tmux pane's `@armyknife-last-claude-code-session-id` user option.
    pub session_id: Option<String>,
}

/// Runs the resume command.
/// If a session ID argument is provided, resumes that session directly.
/// Otherwise, reads the session ID from the current tmux pane's user option.
pub fn run(args: &ResumeArgs) -> Result<()> {
    let session_id = match args.session_id.as_deref() {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => resolve_session_id_from_pane()?,
    };

    let claude_path = find_command_path("claude")
        .ok_or_else(|| anyhow::anyhow!("Could not find 'claude' command in PATH"))?;

    // Serialize claude startup across panes.
    let lock = match acquire_resume_lock() {
        Ok(lock) => Some(lock),
        Err(e) => {
            eprintln!(
                "warning: failed to acquire cc resume lock, proceeding without serialization: {e}"
            );
            None
        }
    };
    if lock.is_some() {
        std::thread::sleep(LOCK_HOLD);
    }
    drop(lock);

    let err = process::exec_replace(&claude_path, ["--resume", &session_id]);
    bail!("Failed to exec claude: {}", err)
}

fn resolve_session_id_from_pane() -> Result<String> {
    let session_id = tmux::get_current_pane_option(TMUX_SESSION_OPTION).ok_or_else(|| {
        anyhow::anyhow!(
            "No Claude Code session ID found for this pane (option '{}' not set)",
            TMUX_SESSION_OPTION
        )
    })?;

    if session_id.is_empty() {
        bail!(
            "No Claude Code session ID found for this pane (option '{}' is empty)",
            TMUX_SESSION_OPTION
        );
    }

    Ok(session_id)
}

/// Returns the lock file path: `~/.cache/armyknife/cc/resume.lock`.
fn resume_lock_path() -> Result<PathBuf> {
    cache::base_dir()
        .map(|d| d.join("cc").join("resume.lock"))
        .ok_or_else(|| CcError::CacheDirNotFound.into())
}

fn acquire_resume_lock() -> Result<File> {
    acquire_exclusive_lock_at(&resume_lock_path()?, LOCK_TIMEOUT, LOCK_RETRY_DELAY)
}

fn acquire_exclusive_lock_at(
    path: &Path,
    timeout: Duration,
    retry_delay: Duration,
) -> Result<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;

    let deadline = std::time::Instant::now() + timeout;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(TryLockError::WouldBlock) => {
                if std::time::Instant::now() >= deadline {
                    return Err(CcError::LockTimeout(timeout).into());
                }
                std::thread::sleep(retry_delay);
            }
            Err(TryLockError::Error(e)) => return Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;
    use std::time::{Duration, Instant};

    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::*;

    #[fixture]
    fn tmpdir() -> TempDir {
        TempDir::new().expect("create tempdir")
    }

    #[rstest]
    fn acquire_creates_parent_and_returns_lock(tmpdir: TempDir) {
        let path = tmpdir.path().join("nested").join("resume.lock");
        let result =
            acquire_exclusive_lock_at(&path, Duration::from_secs(1), Duration::from_millis(10));
        assert_eq!(
            (
                result.is_ok(),
                path.exists(),
                path.parent().map(Path::exists)
            ),
            (true, true, Some(true)),
        );
    }

    #[rstest]
    fn acquire_times_out_when_contended(tmpdir: TempDir) {
        let path = tmpdir.path().join("resume.lock");
        let holder = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .expect("open holder");
        holder.try_lock().expect("acquire initial lock");

        let timeout = Duration::from_millis(100);
        let start = Instant::now();
        let result = acquire_exclusive_lock_at(&path, timeout, Duration::from_millis(20));
        let elapsed = start.elapsed();

        let err = result.expect_err("expected timeout");
        let kind = err.downcast_ref::<CcError>();
        assert_eq!(
            (
                matches!(kind, Some(CcError::LockTimeout(t)) if *t == timeout),
                elapsed >= timeout,
            ),
            (true, true),
        );
    }

    #[rstest]
    fn second_acquire_succeeds_after_first_release(tmpdir: TempDir) {
        let path = tmpdir.path().join("resume.lock");
        {
            let _lock =
                acquire_exclusive_lock_at(&path, Duration::from_secs(1), Duration::from_millis(10))
                    .expect("first acquire");
        }
        let result =
            acquire_exclusive_lock_at(&path, Duration::from_secs(1), Duration::from_millis(10));
        assert_eq!((result.is_ok(), path.exists()), (true, true));
    }
}
