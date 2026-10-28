use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail, ensure};
use rusqlite::Connection;
use serde_json::Value;

use super::super::super::super::history::CodexThreadMetadata;
use super::database;

const MAX_LINEAGE_DEPTH: usize = 128;
const MAX_LINEAGE_THREADS: usize = 128;
const MAX_TOTAL_ROLLOUT_BYTES: u64 = 128 * 1024 * 1024;
const COPY_BUFFER_BYTES: usize = 64 * 1024;
const MAX_SESSION_META_BYTES: usize = 1024 * 1024;

pub(super) fn copy_lineage_and_rewrite(
    source_home: &Path,
    private_home: &Path,
    state_database_path: &Path,
    thread: &CodexThreadMetadata,
    deadline: Instant,
) -> Result<()> {
    let source_home = source_home
        .canonicalize()
        .context("resolve source Codex home for rollout history")?;
    let private_home = private_home
        .canonicalize()
        .context("resolve private Codex home for rollout history")?;
    let connection = Connection::open(state_database_path)
        .context("open private Codex state backup for rollout history")?;
    let mut visiting = HashSet::new();
    let mut complete = HashSet::new();
    let mut budget =
        SnapshotBudget::with_limits(MAX_LINEAGE_THREADS, MAX_TOTAL_ROLLOUT_BYTES, deadline);
    copy_thread_lineage(
        &connection,
        &source_home,
        &private_home,
        &thread.session_id,
        Some(thread.session_id.as_str()),
        0,
        &mut visiting,
        &mut complete,
        &mut budget,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "lineage traversal carries shared cycle and copy tracking state"
)]
fn copy_thread_lineage(
    state: &Connection,
    source_home: &Path,
    private_home: &Path,
    thread_id: &str,
    target_thread_id: Option<&str>,
    depth: usize,
    visiting: &mut HashSet<String>,
    complete: &mut HashSet<String>,
    budget: &mut SnapshotBudget,
) -> Result<()> {
    budget.ensure_within_limits()?;
    ensure!(
        depth <= MAX_LINEAGE_DEPTH,
        "Codex rollout history lineage exceeds the supported depth"
    );
    if complete.contains(thread_id) {
        return Ok(());
    }
    ensure!(
        visiting.insert(thread_id.to_string()),
        "Codex rollout history lineage contains a cycle"
    );
    budget.record_thread()?;

    let record = database::read_thread_record(state, thread_id)?;
    ensure!(
        matches!(record.history_mode.as_str(), "legacy" | "paginated"),
        "Codex rollout history has an unsupported history mode"
    );
    if target_thread_id == Some(thread_id) {
        ensure!(
            record.history_mode == "paginated",
            "Codex status fork target history is not paginated"
        );
    }
    let logical_path = PathBuf::from(&record.rollout_path);
    let materialized = copy_rollout_file(source_home, private_home, &logical_path, budget)?;
    let metadata = read_rollout_metadata(&materialized.metadata_path, materialized.compressed)?;
    let payload = metadata
        .get("payload")
        .and_then(Value::as_object)
        .context("Codex rollout metadata has no payload object")?;
    ensure!(
        metadata.get("type").and_then(Value::as_str) == Some("session_meta"),
        "Codex rollout history has an unsupported metadata record"
    );
    ensure!(
        payload.get("id").and_then(Value::as_str) == Some(thread_id),
        "Codex rollout metadata thread ID does not match its database record"
    );

    for parent_id in [
        optional_metadata_id(payload, "forked_from_id")?,
        optional_metadata_id(payload, "parent_thread_id")?,
    ]
    .into_iter()
    .flatten()
    {
        copy_thread_lineage(
            state,
            source_home,
            private_home,
            &parent_id,
            target_thread_id,
            depth + 1,
            visiting,
            complete,
            budget,
        )?;
    }

    database::rewrite_rollout_path(state, thread_id, &materialized.private_rollout_path)?;
    visiting.remove(thread_id);
    complete.insert(thread_id.to_string());
    Ok(())
}

struct MaterializedRollout {
    private_rollout_path: PathBuf,
    metadata_path: PathBuf,
    compressed: bool,
}

struct SnapshotBudget {
    deadline: Instant,
    thread_count: usize,
    copied_bytes: u64,
    copied_paths: HashSet<PathBuf>,
    max_threads: usize,
    max_bytes: u64,
}

impl SnapshotBudget {
    fn with_limits(max_threads: usize, max_bytes: u64, deadline: Instant) -> Self {
        Self {
            deadline,
            thread_count: 0,
            copied_bytes: 0,
            copied_paths: HashSet::new(),
            max_threads,
            max_bytes,
        }
    }

    fn ensure_within_limits(&self) -> Result<()> {
        ensure!(
            Instant::now() < self.deadline,
            "Codex private home snapshot exceeded its time limit"
        );
        Ok(())
    }

    fn record_thread(&mut self) -> Result<()> {
        self.thread_count = self
            .thread_count
            .checked_add(1)
            .context("Codex rollout history thread count overflowed")?;
        ensure!(
            self.thread_count <= self.max_threads,
            "Codex rollout history lineage exceeds the supported thread count"
        );
        Ok(())
    }

    fn remaining_bytes(&self) -> Result<u64> {
        self.ensure_within_limits()?;
        self.max_bytes
            .checked_sub(self.copied_bytes)
            .context("Codex rollout history snapshot exceeded its byte limit")
    }

    fn record_copied_bytes(&mut self, bytes: u64) -> Result<()> {
        self.copied_bytes = self
            .copied_bytes
            .checked_add(bytes)
            .context("Codex rollout history byte count overflowed")?;
        ensure!(
            self.copied_bytes <= self.max_bytes,
            "Codex rollout history snapshot exceeded its byte limit"
        );
        self.ensure_within_limits()
    }
}

fn copy_rollout_file(
    source_home: &Path,
    private_home: &Path,
    logical_path: &Path,
    budget: &mut SnapshotBudget,
) -> Result<MaterializedRollout> {
    ensure!(
        logical_path.is_absolute(),
        "Codex rollout path is not absolute"
    );
    ensure!(
        logical_path
            .extension()
            .and_then(|extension| extension.to_str())
            == Some("jsonl"),
        "Codex rollout path has an unsupported format"
    );
    ensure!(
        !logical_path
            .components()
            .any(|component| component == Component::ParentDir),
        "Codex rollout path contains a parent traversal"
    );

    let plain = resolve_rollout_candidate_optional(source_home, logical_path)?;
    let compressed_path = logical_path.with_extension("jsonl.zst");
    let compressed = resolve_rollout_candidate_optional(source_home, &compressed_path)?;
    ensure!(
        plain.is_some() || compressed.is_some(),
        "Codex rollout history file is missing"
    );

    let source_metadata_path = plain
        .as_ref()
        .or(compressed.as_ref())
        .map(|path| path.path.clone())
        .context("Codex rollout metadata source is missing")?;
    let selected_compressed = plain.is_none();
    let source_rollout_path = if selected_compressed {
        plain_rollout_path(&source_metadata_path)?
    } else {
        source_metadata_path.clone()
    };
    let relative_path = source_rollout_path
        .strip_prefix(source_home)
        .context("Codex rollout history is outside its home")?;
    validate_rollout_relative_path(relative_path)?;
    let private_rollout_path = private_home.join(relative_path);
    let private_directory = private_rollout_path
        .parent()
        .context("private Codex rollout path has no parent directory")?;
    fs::create_dir_all(private_directory).context("create private Codex rollout directory")?;
    let canonical_directory = private_directory
        .canonicalize()
        .context("resolve private Codex rollout directory")?;
    ensure!(
        canonical_directory.starts_with(private_home),
        "private Codex rollout directory escaped its home"
    );

    if budget.copied_paths.insert(private_rollout_path.clone()) {
        if let Some(plain) = plain {
            copy_file(&plain.path, &private_rollout_path, budget)?;
        }
        if let Some(compressed) = compressed {
            let destination = private_rollout_path.with_extension("jsonl.zst");
            copy_file(&compressed.path, &destination, budget)?;
        }
    }

    Ok(MaterializedRollout {
        private_rollout_path,
        metadata_path: source_metadata_path,
        compressed: selected_compressed,
    })
}

fn plain_rollout_path(compressed_path: &Path) -> Result<PathBuf> {
    let file_name = compressed_path
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .context("Codex compressed rollout filename is not valid UTF-8")?;
    let plain_file_name = file_name
        .strip_suffix(".zst")
        .context("Codex compressed rollout has an unsupported filename")?;
    Ok(compressed_path.with_file_name(plain_file_name))
}

struct RolloutCandidate {
    path: PathBuf,
}

fn resolve_rollout_candidate_optional(
    source_home: &Path,
    path: &Path,
) -> Result<Option<RolloutCandidate>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("inspect source Codex rollout history"),
    };
    ensure!(
        metadata.file_type().is_file(),
        "source Codex rollout history is not a regular file"
    );
    let canonical_path = path
        .canonicalize()
        .context("resolve source Codex rollout history")?;
    let relative_path = canonical_path
        .strip_prefix(source_home)
        .context("source Codex rollout history is outside its home")?;
    validate_rollout_relative_path(relative_path)?;
    Ok(Some(RolloutCandidate {
        path: canonical_path,
    }))
}

fn validate_rollout_relative_path(path: &Path) -> Result<()> {
    let mut components = path.components();
    let area = components
        .next()
        .and_then(|component| component.as_os_str().to_str())
        .context("Codex rollout history has an unsupported path")?;
    ensure!(
        matches!(area, "sessions" | "archived_sessions"),
        "Codex rollout history is outside its managed directories"
    );
    ensure!(
        components.all(|component| matches!(component, Component::Normal(_))),
        "Codex rollout history has an unsupported path"
    );
    Ok(())
}

fn copy_file(source: &Path, destination: &Path, budget: &mut SnapshotBudget) -> Result<()> {
    ensure!(
        !destination.exists(),
        "private Codex rollout destination already exists"
    );
    let remaining = budget.remaining_bytes()?;
    let metadata = fs::metadata(source).context("inspect source Codex rollout history size")?;
    ensure!(
        metadata.len() <= remaining,
        "Codex rollout history snapshot exceeded its byte limit"
    );
    let mut input = File::open(source).context("open source Codex rollout history")?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .context("create private Codex rollout history")?;
    let mut buffer = [0u8; COPY_BUFFER_BYTES];
    let mut copied = 0u64;
    loop {
        budget.ensure_within_limits()?;
        let read = input
            .read(&mut buffer)
            .context("read source Codex rollout history")?;
        if read == 0 {
            break;
        }
        let next_total = copied
            .checked_add(read as u64)
            .context("Codex rollout history byte count overflowed")?;
        ensure!(
            next_total <= remaining,
            "Codex rollout history snapshot exceeded its byte limit"
        );
        output
            .write_all(&buffer[..read])
            .context("write private Codex rollout history")?;
        copied = next_total;
    }
    budget.record_copied_bytes(copied)?;
    Ok(())
}

fn read_rollout_metadata(path: &Path, compressed: bool) -> Result<Value> {
    let file = File::open(path).context("open Codex rollout metadata")?;
    if compressed {
        let decoder =
            zstd::stream::read::Decoder::new(file).context("open compressed Codex rollout")?;
        read_metadata_line(decoder)
    } else {
        read_metadata_line(file)
    }
}

fn read_metadata_line(reader: impl Read) -> Result<Value> {
    let mut reader = BufReader::new(reader.take((MAX_SESSION_META_BYTES + 1) as u64));
    let mut line = Vec::new();
    reader
        .read_until(b'\n', &mut line)
        .context("read Codex rollout metadata line")?;
    ensure!(
        !line.is_empty() && line.len() <= MAX_SESSION_META_BYTES,
        "Codex rollout metadata line is empty or too large"
    );
    serde_json::from_slice(&line).context("parse Codex rollout metadata line")
}

fn optional_metadata_id(
    payload: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<Option<String>> {
    match payload.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.is_empty() => Ok(Some(value.clone())),
        Some(_) => bail!("Codex rollout metadata has an unsupported lineage reference"),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{Duration, Instant};

    use std::path::Path;
    use tempfile::TempDir;

    use super::{SnapshotBudget, copy_file, validate_rollout_relative_path};

    #[test]
    fn accepts_only_managed_rollout_directories() {
        let actual = [
            Path::new("sessions/2026/01/02/fixture.jsonl"),
            Path::new("archived_sessions/2026/01/02/fixture.jsonl"),
            Path::new("external/2026/01/02/fixture.jsonl"),
        ]
        .map(|path| validate_rollout_relative_path(path).is_ok());
        assert_eq!(actual, [true, true, false]);
    }

    #[test]
    fn bounds_lineage_threads_and_copied_bytes() -> anyhow::Result<()> {
        let root = TempDir::new()?;
        let source = root.path().join("source-rollout.jsonl");
        let destination = root.path().join("private-rollout.jsonl");
        fs::write(&source, b"two bytes")?;

        let mut budget = SnapshotBudget::with_limits(1, 1, Instant::now() + Duration::from_secs(5));
        let first_thread = budget.record_thread().map(|_| ());
        let second_thread = budget.record_thread().map(|_| ());
        let copy_error = copy_file(&source, &destination, &mut budget)
            .expect_err("oversized history should fail closed")
            .to_string();

        assert_eq!(
            (
                first_thread.is_ok(),
                second_thread.unwrap_err().to_string(),
                copy_error,
                destination.exists()
            ),
            (
                true,
                "Codex rollout history lineage exceeds the supported thread count".to_string(),
                "Codex rollout history snapshot exceeded its byte limit".to_string(),
                false,
            ),
        );
        Ok(())
    }
}
