use std::fs::{self, File, OpenOptions, TryLockError};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use clap::Args;

use super::error::CcError;
use super::types::TMUX_SESSION_OPTION;
use crate::infra::tmux;
use crate::shared::cache;
use crate::shared::command::{self, find_command_path};
use crate::shared::env_var::EnvVars;

const LOCK_RETRY_DELAY: Duration = Duration::from_millis(100);
const LOCK_TIMEOUT: Duration = Duration::from_secs(300);

const CLAIM_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Force-clear the claim after this duration. Triggers when claude crashes
/// before its SessionStart hook fires, or when the user has no armyknife
/// SessionStart hook configured.
const CLAIM_STALE_AFTER: Duration = Duration::from_secs(60);

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

    let claim_path = resume_claim_path()?;
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
        wait_for_prior_claim(&claim_path);
        if let Err(e) = create_resume_claim(&claim_path) {
            eprintln!("warning: failed to create cc resume claim: {e}");
        }
    }
    drop(lock);

    let err = command::new(&claude_path)
        .args(["--resume", &session_id])
        .env(EnvVars::resume_ack_name(), &claim_path)
        .exec();
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

/// Returns the claim sentinel path: `~/.cache/armyknife/cc/resume.claim`.
/// Created by `a cc resume`; deleted by `a cc hook` SessionStart to signal
/// that claude has past its racy session-id read.
fn resume_claim_path() -> Result<PathBuf> {
    cache::base_dir()
        .map(|d| d.join("cc").join("resume.claim"))
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

    let deadline = Instant::now() + timeout;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(TryLockError::WouldBlock) => {
                if Instant::now() >= deadline {
                    return Err(CcError::LockTimeout(timeout).into());
                }
                std::thread::sleep(retry_delay);
            }
            Err(TryLockError::Error(e)) => return Err(e.into()),
        }
    }
}

fn wait_for_prior_claim(path: &Path) {
    wait_for_prior_claim_with(path, CLAIM_POLL_INTERVAL, CLAIM_STALE_AFTER);
}

fn wait_for_prior_claim_with(path: &Path, poll: Duration, stale_after: Duration) {
    while path.exists() {
        if let Ok(meta) = fs::metadata(path)
            && let Ok(modified) = meta.modified()
            && modified.elapsed().unwrap_or_default() > stale_after
        {
            let _ = fs::remove_file(path);
            return;
        }
        std::thread::sleep(poll);
    }
}

fn create_resume_claim(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, std::process::id().to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;
    use std::time::{Duration, Instant};

    use filetime::{FileTime, set_file_mtime};
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
    fn wait_returns_immediately_when_no_claim(tmpdir: TempDir) {
        let path = tmpdir.path().join("resume.claim");
        let start = Instant::now();
        wait_for_prior_claim_with(&path, Duration::from_millis(50), Duration::from_secs(60));
        assert!(start.elapsed() < Duration::from_millis(50));
    }

    #[rstest]
    fn wait_returns_when_claim_removed(tmpdir: TempDir) {
        let path = tmpdir.path().join("resume.claim");
        fs::write(&path, "owner").expect("write claim");
        let path_clone = path.clone();
        let removed_at = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(80));
            fs::remove_file(&path_clone).expect("remove");
            Instant::now()
        });
        let start = Instant::now();
        wait_for_prior_claim_with(&path, Duration::from_millis(20), Duration::from_secs(60));
        let waited = start.elapsed();
        removed_at.join().expect("join");
        assert_eq!(
            (waited >= Duration::from_millis(80), path.exists()),
            (true, false),
        );
    }

    #[rstest]
    fn wait_force_clears_stale_claim(tmpdir: TempDir) {
        let path = tmpdir.path().join("resume.claim");
        fs::write(&path, "stale").expect("write claim");
        let two_hours_ago = std::time::SystemTime::now() - Duration::from_secs(7200);
        set_file_mtime(&path, FileTime::from_system_time(two_hours_ago)).expect("set mtime");

        wait_for_prior_claim_with(&path, Duration::from_millis(10), Duration::from_secs(60));
        assert!(!path.exists());
    }

    #[rstest]
    fn create_resume_claim_writes_pid(tmpdir: TempDir) {
        let path = tmpdir.path().join("nested").join("resume.claim");
        create_resume_claim(&path).expect("create");
        let content = fs::read_to_string(&path).expect("read");
        assert_eq!(content, std::process::id().to_string());
    }
}
