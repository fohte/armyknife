use std::fs::{self, File};
use std::io::{Seek, SeekFrom, copy};
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tempfile::NamedTempFile;

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
    reconcile_in_flight_appends(
        old_inode_path,
        original_source,
        staged_appends.as_file_mut(),
        copied_to,
        has_writable_open_handle,
        remaining(deadline)?,
    )?;

    loop {
        if staged_appends.as_file().metadata()?.len() == 0 {
            return Ok(());
        }
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
            .with_context(|| {
                format!("replace transcript {} with merged contents", path.display())
            })?;
        drop(persisted_file);

        let mut next_staged_appends = NamedTempFile::new_in(parent)
            .with_context(|| format!("stage late transcript appends in {}", parent.display()))?;
        reconcile_in_flight_appends(
            monitor.path(),
            &mut live_source,
            next_staged_appends.as_file_mut(),
            copied_live_bytes,
            has_writable_open_handle,
            remaining(deadline)?,
        )?;
        staged_appends = next_staged_appends;
    }
}

fn remaining(deadline: Instant) -> Result<Duration> {
    let now = Instant::now();
    if now >= deadline {
        bail!("timed out reconciling post-replacement transcript appends");
    }
    Ok(deadline.duration_since(now))
}
