use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use super::{
    PostReplacementFailure, SlimArgs, SlimResult, TranscriptCommands, slim_transcript,
    validate_session_id,
};

const LOCK_DIRECTORY: &str = "agent-transcript-slim";

pub(super) fn run(command: &TranscriptCommands) -> Result<()> {
    match command {
        TranscriptCommands::Slim(args) => match slim_session_transcript(args) {
            Ok(result) => {
                tracing::info!(
                    event = "agent.transcript.slim.completed",
                    session = %args.session_id,
                    original_bytes = result.original_bytes,
                    final_bytes = result.final_bytes,
                );
                println!(
                    "Slimmed transcript: {} -> {} bytes ({} bytes removed)",
                    result.original_bytes,
                    result.final_bytes,
                    result.original_bytes.saturating_sub(result.final_bytes)
                );
            }
            Err(error) => {
                tracing::warn!(
                    event = slim_failure_event(&error),
                    session = %args.session_id,
                    error = %format!("{error:#}"),
                );
                return Err(error);
            }
        },
    }
    Ok(())
}

fn slim_session_transcript(args: &SlimArgs) -> Result<SlimResult> {
    validate_session_id(&args.session_id)?;
    let path = resolve_transcript_path(args)?;
    let _locks = lock_transcript(&args.session_id, &path)?;
    slim_transcript(&path)
}

fn resolve_transcript_path(args: &SlimArgs) -> Result<PathBuf> {
    if let Some(path) = &args.transcript_path {
        return Ok(path.clone());
    }

    let cwd = fs::canonicalize(&args.cwd)
        .with_context(|| format!("resolve project directory {}", args.cwd.display()))?;
    super::super::super::claude_sessions::session_jsonl_path(&cwd, &args.session_id)
        .context("resolve Claude Code home directory")
}

fn lock_transcript(session_id: &str, transcript_path: &Path) -> Result<(File, File)> {
    let cache_dir =
        crate::shared::cache::base_dir().context("resolve armyknife cache directory")?;
    let canonical_path = fs::canonicalize(transcript_path).with_context(|| {
        format!(
            "resolve transcript path for locking {}",
            transcript_path.display()
        )
    })?;
    let session_lock = lock_transcript_session_in(&cache_dir, session_id)?;
    let transcript_lock = lock_transcript_path_in(&cache_dir, &canonical_path)?;
    Ok((session_lock, transcript_lock))
}

fn lock_transcript_session_in(cache_dir: &Path, session_id: &str) -> Result<File> {
    let digest = crate::shared::hex::encode(&Sha256::digest(session_id.as_bytes()));
    lock_transcript_in(cache_dir, &digest)
}

fn lock_transcript_path_in(cache_dir: &Path, transcript_path: &Path) -> Result<File> {
    let mut hasher = Sha256::new();
    hasher.update(b"transcript-path\0");
    hasher.update(transcript_path.as_os_str().as_encoded_bytes());
    let digest = crate::shared::hex::encode(&hasher.finalize());
    lock_transcript_in(cache_dir, &digest)
}

fn lock_transcript_in(cache_dir: &Path, digest: &str) -> Result<File> {
    let lock_dir = cache_dir.join(LOCK_DIRECTORY);
    fs::create_dir_all(&lock_dir).with_context(|| {
        format!(
            "create transcript slim lock directory {}",
            lock_dir.display()
        )
    })?;
    let lock_path = lock_dir.join(format!("{digest}.lock"));
    let lock_file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("open transcript slim lock {}", lock_path.display()))?;
    lock_file
        .lock()
        .with_context(|| format!("lock transcript slim resource {}", lock_path.display()))?;
    Ok(lock_file)
}

fn slim_failure_event(error: &anyhow::Error) -> &'static str {
    if error
        .chain()
        .any(|cause| cause.downcast_ref::<PostReplacementFailure>().is_some())
    {
        "agent.transcript.slim.post_replacement_failed"
    } else {
        "agent.transcript.slim.failed"
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{self, OpenOptions};
    use std::path::Path;

    use rstest::rstest;
    use tempfile::tempdir;

    use super::{
        LOCK_DIRECTORY, PostReplacementFailure, SlimArgs, lock_transcript_path_in,
        lock_transcript_session_in, resolve_transcript_path, slim_failure_event,
    };

    #[test]
    fn session_lock_prevents_a_second_slim_from_running_concurrently() {
        let directory = tempdir().expect("lock root should be created");
        let first_lock = lock_transcript_session_in(directory.path(), "fixture-session")
            .expect("first lock should be acquired");
        let lock_path = fs::read_dir(directory.path().join("agent-transcript-slim"))
            .expect("lock directory should be readable")
            .next()
            .expect("session lock should exist")
            .expect("session lock entry should be readable")
            .path();
        let second_lock = OpenOptions::new()
            .write(true)
            .open(lock_path)
            .expect("second lock handle should open");

        assert!(
            second_lock
                .try_lock()
                .is_err_and(|error| matches!(error, std::fs::TryLockError::WouldBlock))
        );
        drop(first_lock);
    }

    #[test]
    fn transcript_path_lock_prevents_a_second_worker_for_the_same_file() {
        let directory = tempdir().expect("lock root should be created");
        let transcript_path = directory.path().join("fixture-transcript.jsonl");
        let first_lock = lock_transcript_path_in(directory.path(), &transcript_path)
            .expect("first transcript lock should be acquired");
        let lock_path = fs::read_dir(directory.path().join(LOCK_DIRECTORY))
            .expect("lock directory should be readable")
            .next()
            .expect("transcript lock should exist")
            .expect("transcript lock entry should be readable")
            .path();
        let second_lock = OpenOptions::new()
            .write(true)
            .open(lock_path)
            .expect("second transcript lock handle should open");

        assert!(
            second_lock
                .try_lock()
                .is_err_and(|error| matches!(error, std::fs::TryLockError::WouldBlock))
        );
        drop(first_lock);
    }

    #[test]
    fn hook_transcript_path_takes_precedence_over_project_directory() {
        let args = SlimArgs {
            session_id: "fixture-session".to_string(),
            cwd: Path::new("/fixture/other-project").to_path_buf(),
            transcript_path: Some(
                Path::new("/fixture/transcripts/fixture-session.jsonl").to_path_buf(),
            ),
        };

        assert_eq!(
            resolve_transcript_path(&args).expect("hook transcript path should be used"),
            Path::new("/fixture/transcripts/fixture-session.jsonl"),
        );
    }

    #[rstest]
    #[case::before_replacement(
        "failed to inspect transcript",
        false,
        "agent.transcript.slim.failed"
    )]
    #[case::after_replacement(
        "post-replacement reconciliation failed",
        true,
        "agent.transcript.slim.post_replacement_failed"
    )]
    fn slim_failures_are_logged_by_replacement_state(
        #[case] message: &str,
        #[case] post_replacement: bool,
        #[case] expected_event: &str,
    ) {
        let inner_error = anyhow::anyhow!("{message}");
        let error: anyhow::Error = if post_replacement {
            PostReplacementFailure {
                message: message.to_string(),
                source: inner_error,
            }
            .into()
        } else {
            inner_error
        };

        assert_eq!(slim_failure_event(&error), expected_event);
    }
}
