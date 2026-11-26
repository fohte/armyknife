use std::fs::OpenOptions;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write, copy};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tempfile::{Builder, NamedTempFile};

use super::{FileIdentity, OldInodeMonitor, copy_complete_lines, reconcile_in_flight_appends};

pub(super) fn reconcile_after_replacement<P>(
    path: &Path,
    old_inode_path: &Path,
    original_source: &mut File,
    copied_to: u64,
    has_writable_open_handle: &mut P,
    max_wait: Duration,
) -> Result<()>
where
    P: FnMut(&Path) -> Result<bool>,
{
    let deadline = Instant::now() + max_wait;
    let parent = path
        .parent()
        .context("Claude Code transcript has no parent directory")?;
    let mut staged_appends = NamedTempFile::new_in(parent)
        .with_context(|| format!("stage in-flight transcript appends in {}", parent.display()))?;
    let initial_reconciliation = reconcile_in_flight_appends(
        old_inode_path,
        original_source,
        staged_appends.as_file_mut(),
        copied_to,
        has_writable_open_handle,
        remaining(deadline)?,
    );
    if let Err(error) = initial_reconciliation {
        preserve_staged_appends(path, staged_appends).with_context(|| {
            format!("preserve staged rows after reconciliation failure: {error:#}")
        })?;
        return Err(error);
    }

    loop {
        if staged_appends.as_file().metadata()?.len() == 0 {
            return Ok(());
        }
        let merged = merge_staged_appends(path, parent, &mut staged_appends);
        let (mut live_source, monitor, copied_live_bytes) = match merged {
            Ok(merged) => merged,
            Err(error) => {
                preserve_staged_appends(path, staged_appends).with_context(|| {
                    format!("preserve staged rows after merge failure: {error:#}")
                })?;
                return Err(error);
            }
        };

        staged_appends
            .as_file_mut()
            .set_len(0)
            .context("reset staged transcript appends after merge")?;
        staged_appends
            .as_file_mut()
            .seek(SeekFrom::Start(0))
            .context("rewind staged transcript appends after merge")?;

        let reconciliation = reconcile_in_flight_appends(
            monitor.path(),
            &mut live_source,
            staged_appends.as_file_mut(),
            copied_live_bytes,
            has_writable_open_handle,
            remaining(deadline)?,
        );
        if let Err(error) = reconciliation {
            preserve_staged_appends(path, staged_appends).with_context(|| {
                format!("preserve staged rows after reconciliation failure: {error:#}")
            })?;
            return Err(error);
        }
    }
}

fn merge_staged_appends(
    path: &Path,
    parent: &Path,
    staged_appends: &mut NamedTempFile,
) -> Result<(File, OldInodeMonitor, u64)> {
    let mut live_source = File::open(path).with_context(|| {
        format!(
            "open replaced transcript {} for reconciliation",
            path.display()
        )
    })?;
    let metadata = live_source
        .metadata()
        .with_context(|| format!("inspect replaced transcript {}", path.display()))?;
    let identity = FileIdentity::from_metadata(&metadata);
    let monitor = OldInodeMonitor::new(path)?;
    let mut replacement = NamedTempFile::new_in(parent)
        .with_context(|| format!("create merged transcript in {}", parent.display()))?;
    let copied_live_bytes = copy_complete_lines(
        &mut live_source,
        metadata.len(),
        0,
        replacement.as_file_mut(),
    )?;
    staged_appends
        .as_file_mut()
        .seek(SeekFrom::Start(0))
        .context("rewind staged transcript appends")?;
    copy(staged_appends.as_file_mut(), replacement.as_file_mut())
        .context("merge staged transcript appends")?;
    replacement
        .as_file()
        .sync_all()
        .context("sync merged transcript replacement")?;
    fs::set_permissions(replacement.path(), metadata.permissions())
        .with_context(|| format!("preserve transcript permissions for {}", path.display()))?;
    if !identity.matches_path(path)? {
        bail!("transcript was replaced while it was being slimmed");
    }
    let persisted_file = replacement
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("replace transcript {} with merged contents", path.display()))?;
    drop(persisted_file);
    Ok((live_source, monitor, copied_live_bytes))
}

fn preserve_staged_appends(path: &Path, mut staged_appends: NamedTempFile) -> Result<()> {
    if staged_appends.as_file().metadata()?.len() == 0 {
        return Ok(());
    }

    match append_staged_appends(path, &mut staged_appends) {
        Ok(None) => Ok(()),
        Ok(Some(incomplete_row)) => match save_incomplete_row(path, &incomplete_row) {
            Ok(recovery_path) => {
                bail!(
                    "incomplete appended transcript row saved to {}",
                    recovery_path.display()
                );
            }
            Err(error) => {
                let (_file, recovery_path) = staged_appends
                    .keep()
                    .map_err(|error| error.error)
                    .context("retain staged transcript rows for recovery")?;
                Err(error).with_context(|| {
                    format!(
                        "staged transcript rows were saved to {}",
                        recovery_path.display()
                    )
                })
            }
        },
        Err(error) => {
            let (_file, recovery_path) = staged_appends
                .keep()
                .map_err(|error| error.error)
                .context("retain staged transcript rows for recovery")?;
            Err(error).with_context(|| {
                format!(
                    "staged transcript rows were saved to {}",
                    recovery_path.display()
                )
            })
        }
    }
}

fn append_staged_appends(
    path: &Path,
    staged_appends: &mut NamedTempFile,
) -> Result<Option<Vec<u8>>> {
    let mut transcript = OpenOptions::new()
        .read(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open transcript {} to preserve staged rows", path.display()))?;
    let transcript_len = transcript
        .metadata()
        .with_context(|| format!("inspect transcript {} before staged append", path.display()))?
        .len();
    if transcript_len > 0 {
        transcript
            .seek(SeekFrom::End(-1))
            .context("inspect final transcript byte before staged append")?;
        let mut last_byte = [0_u8; 1];
        transcript
            .read_exact(&mut last_byte)
            .context("read final transcript byte before staged append")?;
        if last_byte[0] != b'\n' {
            bail!(
                "transcript has an incomplete trailing row; staged rows cannot be appended safely"
            );
        }
    }

    staged_appends
        .as_file_mut()
        .seek(SeekFrom::Start(0))
        .context("rewind staged transcript rows")?;
    let mut reader = BufReader::new(staged_appends.as_file_mut());
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = reader
            .read_until(b'\n', &mut line)
            .context("read staged transcript row")?;
        if read == 0 {
            return Ok(None);
        }
        if line.last() != Some(&b'\n') {
            return Ok(Some(line));
        }
        // Claude Code appends each JSONL record with one O_APPEND write.
        let written = transcript
            .write(&line)
            .context("append staged transcript row")?;
        if written != read {
            bail!("short write while appending staged transcript row");
        }
    }
}

fn save_incomplete_row(path: &Path, row: &[u8]) -> Result<PathBuf> {
    let parent = path
        .parent()
        .context("Claude Code transcript has no parent directory")?;
    let mut recovery = Builder::new()
        .prefix(".armyknife-transcript-slim-recovery-")
        .suffix(".jsonl-fragment")
        .tempfile_in(parent)
        .with_context(|| format!("create transcript recovery file in {}", parent.display()))?;
    recovery
        .write_all(row)
        .context("write incomplete transcript row recovery file")?;
    recovery
        .as_file()
        .sync_all()
        .context("sync incomplete transcript row recovery file")?;
    let (_file, recovery_path) = recovery
        .keep()
        .map_err(|error| error.error)
        .context("retain incomplete transcript row recovery file")?;
    Ok(recovery_path)
}

fn remaining(deadline: Instant) -> Result<Duration> {
    let now = Instant::now();
    if now >= deadline {
        bail!("timed out reconciling post-replacement transcript appends");
    }
    Ok(deadline.duration_since(now))
}
