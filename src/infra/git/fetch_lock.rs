//! Serializes and coalesces `git fetch` against a shared repository.
//!
//! Concurrent `git fetch` invocations on the same repo race on
//! `refs/remotes/origin/*` lock files and one or more typically fail with
//! `cannot lock ref ...`.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;

use super::error::Result;

/// Window during which a completed fetch is treated as fresh enough to skip
/// for a subsequent caller.
pub const FETCH_TTL: Duration = Duration::from_secs(30);

/// Acquire an exclusive flock on `lock_path`, run `fetch_fn` only if the last
/// recorded fetch is older than `ttl`, then persist the new timestamp before
/// releasing the lock.
pub fn fetch_with_coalescing<F>(lock_path: &Path, ttl: Duration, fetch_fn: F) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .with_context(|| format!("Failed to open fetch lock at {}", lock_path.display()))?;

    let _guard = FlockGuard::acquire(&file)?;

    let last = read_timestamp(&file)?;
    if should_skip_fetch(last, SystemTime::now(), ttl) {
        return Ok(());
    }

    fetch_fn()?;

    write_timestamp(&file, SystemTime::now())?;
    Ok(())
}

fn should_skip_fetch(last: Option<SystemTime>, now: SystemTime, ttl: Duration) -> bool {
    let Some(last) = last else { return false };
    match now.duration_since(last) {
        Ok(elapsed) => elapsed < ttl,
        // mtime in the future (clock skew); treat as fresh rather than
        // re-fetching every call until wall-clock catches up.
        Err(_) => true,
    }
}

fn read_timestamp(mut file: &File) -> Result<Option<SystemTime>> {
    file.seek(SeekFrom::Start(0))
        .context("Failed to seek fetch lock")?;
    let mut buf = String::new();
    file.read_to_string(&mut buf)
        .context("Failed to read fetch lock")?;
    let trimmed = buf.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    match trimmed.parse::<u64>() {
        Ok(secs) => Ok(Some(UNIX_EPOCH + Duration::from_secs(secs))),
        // Garbage in the lock file: treat as "no recorded fetch" rather than
        // failing hard, so a corrupted file self-heals on the next fetch.
        Err(_) => Ok(None),
    }
}

fn write_timestamp(mut file: &File, time: SystemTime) -> Result<()> {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    file.seek(SeekFrom::Start(0))
        .context("Failed to seek fetch lock")?;
    file.set_len(0).context("Failed to truncate fetch lock")?;
    write!(file, "{secs}").context("Failed to write fetch lock")?;
    Ok(())
}

struct FlockGuard<'f> {
    file: &'f File,
}

impl<'f> FlockGuard<'f> {
    fn acquire(file: &'f File) -> Result<Self> {
        // SAFETY: `file` is borrowed for the guard's lifetime, so its fd is
        // valid for the duration of the flock call.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if rc != 0 {
            return Err(anyhow::anyhow!(
                "Failed to acquire fetch lock: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(Self { file })
    }
}

impl Drop for FlockGuard<'_> {
    fn drop(&mut self) {
        // SAFETY: `self.file` is borrowed for this guard's lifetime.
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::cell::Cell;
    use std::time::{Duration, UNIX_EPOCH};
    use tempfile::tempdir;

    #[rstest]
    #[case::no_prior_fetch(None, 100, 30, false)]
    #[case::within_ttl(Some(80), 100, 30, true)]
    #[case::exactly_at_boundary(Some(70), 100, 30, false)]
    #[case::past_ttl(Some(50), 100, 30, false)]
    #[case::future_mtime_treated_as_fresh(Some(150), 100, 30, true)]
    fn test_should_skip_fetch(
        #[case] last_secs: Option<u64>,
        #[case] now_secs: u64,
        #[case] ttl_secs: u64,
        #[case] expected: bool,
    ) {
        let last = last_secs.map(|s| UNIX_EPOCH + Duration::from_secs(s));
        let now = UNIX_EPOCH + Duration::from_secs(now_secs);
        let ttl = Duration::from_secs(ttl_secs);
        assert_eq!(should_skip_fetch(last, now, ttl), expected);
    }

    #[test]
    fn test_fetch_runs_on_first_call_and_skips_within_ttl() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("fetch.lock");
        let calls = Cell::new(0u32);
        let ttl = Duration::from_secs(3600);

        fetch_with_coalescing(&path, ttl, || {
            calls.set(calls.get() + 1);
            Ok(())
        })
        .unwrap();
        fetch_with_coalescing(&path, ttl, || {
            calls.set(calls.get() + 1);
            Ok(())
        })
        .unwrap();

        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn test_fetch_runs_every_call_when_ttl_zero() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("fetch.lock");
        let calls = Cell::new(0u32);
        let ttl = Duration::from_secs(0);

        for _ in 0..3 {
            fetch_with_coalescing(&path, ttl, || {
                calls.set(calls.get() + 1);
                Ok(())
            })
            .unwrap();
        }

        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn test_fetch_error_does_not_record_timestamp() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("fetch.lock");
        let calls = Cell::new(0u32);
        let ttl = Duration::from_secs(3600);

        let first = fetch_with_coalescing(&path, ttl, || {
            calls.set(calls.get() + 1);
            Err(anyhow::anyhow!("boom"))
        });
        assert!(first.is_err());

        fetch_with_coalescing(&path, ttl, || {
            calls.set(calls.get() + 1);
            Ok(())
        })
        .unwrap();

        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn test_corrupted_lock_file_self_heals() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("fetch.lock");
        std::fs::write(&path, "not a number").unwrap();
        let calls = Cell::new(0u32);
        let ttl = Duration::from_secs(3600);

        // First call: corrupt content is treated as "no recorded fetch", so
        // fetch_fn runs and a valid timestamp is written. Second call: within
        // TTL of the freshly written timestamp, so fetch_fn must NOT run.
        // Total invocation count of 1 proves the file was self-healed.
        for _ in 0..2 {
            fetch_with_coalescing(&path, ttl, || {
                calls.set(calls.get() + 1);
                Ok(())
            })
            .unwrap();
        }

        assert_eq!(calls.get(), 1);
    }
}
