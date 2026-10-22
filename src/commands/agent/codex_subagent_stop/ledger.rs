use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use super::HookInput;
use crate::shared::cache;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(super) struct EventKey {
    session_id: String,
    child_thread_id: String,
    turn_id: String,
}

impl From<&HookInput> for EventKey {
    fn from(input: &HookInput) -> Self {
        Self {
            session_id: input.session_id.clone(),
            child_thread_id: input.agent_id.clone(),
            turn_id: input.turn_id.clone(),
        }
    }
}

pub(super) trait EventLease {
    fn mark_processed(&mut self) -> Result<()>;
}

pub(super) trait EventLedger {
    fn begin(&self, key: &EventKey) -> Result<Option<Box<dyn EventLease>>>;
}

pub(super) struct FileEventLedger {
    directory: PathBuf,
}

impl FileEventLedger {
    pub(super) fn new() -> Result<Self> {
        let directory = cache::base_dir()
            .context("could not determine armyknife cache directory")?
            .join("agent")
            .join("codex-subagent-stop");
        Self::at_directory(&directory)
    }

    fn at_directory(directory: &Path) -> Result<Self> {
        fs::create_dir_all(directory).with_context(|| {
            format!(
                "failed to create Codex SubagentStop state directory {}",
                directory.display()
            )
        })?;
        Ok(Self {
            directory: directory.to_path_buf(),
        })
    }
}

impl EventLedger for FileEventLedger {
    fn begin(&self, key: &EventKey) -> Result<Option<Box<dyn EventLease>>> {
        let digest = event_key_digest(key);
        let lock_path = self.directory.join(format!("{digest}.lock"));
        let processed_path = self.directory.join(format!("{digest}.processed"));
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&lock_path)
            .with_context(|| format!("failed to open SubagentStop lock {}", lock_path.display()))?;

        // SAFETY: `lock_file` stays open for as long as this lease owns the process lock.
        let locked = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX) };
        if locked != 0 {
            return Err(std::io::Error::last_os_error())
                .with_context(|| format!("failed to lock SubagentStop event {}", digest));
        }

        if processed_path.try_exists().with_context(|| {
            format!(
                "failed to inspect SubagentStop marker {}",
                processed_path.display()
            )
        })? {
            return Ok(None);
        }

        Ok(Some(Box::new(FileEventLease {
            _lock_file: lock_file,
            processed_path,
            processed: false,
        })))
    }
}

struct FileEventLease {
    _lock_file: File,
    processed_path: PathBuf,
    processed: bool,
}

impl EventLease for FileEventLease {
    fn mark_processed(&mut self) -> Result<()> {
        if self.processed {
            return Ok(());
        }
        let mut marker = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&self.processed_path)
            .with_context(|| {
                format!(
                    "failed to write SubagentStop marker {}",
                    self.processed_path.display()
                )
            })?;
        marker.write_all(b"processed\n")?;
        marker.sync_all()?;
        self.processed = true;
        Ok(())
    }
}

fn event_key_digest(key: &EventKey) -> String {
    let mut digest = Sha256::new();
    digest.update(key.session_id.as_bytes());
    digest.update([0]);
    digest.update(key.child_thread_id.as_bytes());
    digest.update([0]);
    digest.update(key.turn_id.as_bytes());
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use rstest::{fixture, rstest};

    use super::{EventKey, EventLedger, FileEventLedger};
    use crate::commands::agent::codex_subagent_stop::HookInput;

    #[fixture]
    fn input() -> HookInput {
        HookInput {
            session_id: "sample-root-session".to_string(),
            agent_id: "sample-child-thread".to_string(),
            turn_id: "sample-child-turn".to_string(),
        }
    }

    #[rstest]
    fn records_a_processed_event_on_disk_for_duplicate_workers(input: HookInput) {
        let directory = tempfile::tempdir().expect("temporary state directory");
        let ledger = FileEventLedger::at_directory(directory.path()).expect("event ledger");
        let key = EventKey::from(&input);
        let mut first = ledger
            .begin(&key)
            .expect("claim succeeds")
            .expect("first worker owns event");
        first.mark_processed().expect("marker is written");
        drop(first);

        let duplicate = ledger.begin(&key).expect("duplicate check succeeds");

        assert_eq!(duplicate.map(|_| "claimed"), None);
    }

    #[rstest]
    fn serializes_concurrent_workers_for_the_same_child_turn(input: HookInput) {
        let directory = tempfile::tempdir().expect("temporary state directory");
        let first_ledger = FileEventLedger::at_directory(directory.path()).expect("first ledger");
        let second_ledger = FileEventLedger::at_directory(directory.path()).expect("second ledger");
        let key = EventKey::from(&input);
        let mut first = first_ledger
            .begin(&key)
            .expect("first claim succeeds")
            .expect("first worker owns event");
        let (attempt_tx, attempt_rx) = mpsc::channel();
        let (claim_tx, claim_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let second_worker = thread::spawn(move || {
            attempt_tx.send(()).expect("signal lock attempt");
            let claim = second_ledger
                .begin(&key)
                .map_err(|error| error.to_string())?;
            let claimed = claim.is_some();
            claim_tx.send(claimed).expect("report claim state");
            if let Some(mut lease) = claim {
                release_rx.recv().expect("release second worker");
                lease.mark_processed().map_err(|error| error.to_string())?;
            }
            Ok::<_, String>(claimed)
        });
        attempt_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("second worker attempts to claim event");
        let overlap_claim = claim_rx.recv_timeout(Duration::from_millis(100));

        first.mark_processed().expect("first worker marks event");
        drop(first);
        let _ = release_tx.send(());
        let second_result = second_worker.join().expect("second worker");

        assert_eq!(
            (overlap_claim, second_result),
            (Err(mpsc::RecvTimeoutError::Timeout), Ok(false)),
        );
    }
}
