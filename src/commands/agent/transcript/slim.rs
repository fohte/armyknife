use std::collections::{HashMap, HashSet};
use std::fs::{self, File, Metadata};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use tempfile::{Builder, NamedTempFile, TempPath};

#[cfg(not(test))]
use crate::shared::command;

use super::super::claude_sessions::session_jsonl_path;

mod transform;

const MAX_TRANSCRIPT_RECONCILE_WAIT: Duration = Duration::from_secs(5);

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum TranscriptCommands {
    /// Slim records before the latest compact boundary
    Slim(SlimArgs),
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct SlimArgs {
    /// Claude Code session ID
    pub session_id: String,

    /// Project directory used to resolve the session transcript
    #[arg(long, default_value = ".")]
    pub cwd: PathBuf,
}

pub fn run(command: &TranscriptCommands) -> Result<()> {
    match command {
        TranscriptCommands::Slim(args) => {
            let cwd = fs::canonicalize(&args.cwd)
                .with_context(|| format!("resolve project directory {}", args.cwd.display()))?;
            validate_session_id(&args.session_id)?;
            let path = session_jsonl_path(&cwd, &args.session_id)
                .context("resolve Claude Code home directory")?;
            let result = slim_transcript(&path)?;
            println!(
                "Slimmed transcript: {} -> {} bytes ({} bytes removed)",
                result.original_bytes,
                result.final_bytes,
                result.original_bytes.saturating_sub(result.final_bytes)
            );
        }
    }
    Ok(())
}

fn validate_session_id(session_id: &str) -> Result<()> {
    let path = Path::new(session_id);
    if session_id.is_empty()
        || path.components().count() != 1
        || path.file_name().and_then(|name| name.to_str()) != Some(session_id)
    {
        bail!("session ID must be a single path component");
    }
    Ok(())
}

struct SlimResult {
    original_bytes: u64,
    final_bytes: u64,
}

struct TranscriptAnalysis {
    boundary_offset: Option<u64>,
    preserved_uuids: HashSet<String>,
    read_paths_by_tool_use: HashMap<String, String>,
}

#[cfg(not(test))]
fn slim_transcript(path: &Path) -> Result<SlimResult> {
    slim_transcript_with(
        path,
        || {},
        || {},
        || {},
        has_writable_open_handle,
        MAX_TRANSCRIPT_RECONCILE_WAIT,
    )
}

#[cfg(test)]
fn slim_transcript(path: &Path) -> Result<SlimResult> {
    slim_transcript_with(
        path,
        || {},
        || {},
        || {},
        |_| Ok(false),
        MAX_TRANSCRIPT_RECONCILE_WAIT,
    )
}

// The compact hook must serialize same-session invocations to prevent competing replacements.
fn slim_transcript_with<F, G, H, P>(
    path: &Path,
    during_rewrite: F,
    before_replace: G,
    after_replace: H,
    mut has_writable_open_handle: P,
    max_reconcile_wait: Duration,
) -> Result<SlimResult>
where
    F: FnOnce(),
    G: FnOnce(),
    H: FnOnce(),
    P: FnMut(&Path) -> Result<bool>,
{
    let mut source = File::open(path)
        .with_context(|| format!("open Claude Code transcript {}", path.display()))?;
    let metadata = source
        .metadata()
        .with_context(|| format!("inspect Claude Code transcript {}", path.display()))?;
    let original_bytes = metadata.len();
    let analysis = transform::analyze_transcript(&mut source, original_bytes)
        .with_context(|| format!("analyze Claude Code transcript {}", path.display()))?;
    let Some(boundary_offset) = analysis.boundary_offset else {
        return Ok(SlimResult {
            original_bytes,
            final_bytes: original_bytes,
        });
    };

    let parent = path
        .parent()
        .context("Claude Code transcript has no parent directory")?;
    let mut replacement = NamedTempFile::new_in(parent)
        .with_context(|| format!("create replacement transcript in {}", parent.display()))?;
    let copied_to = transform::write_slimmed_prefix(
        &mut source,
        original_bytes,
        boundary_offset,
        &analysis,
        &mut replacement,
    )
    .with_context(|| format!("write slimmed transcript {}", path.display()))?;

    during_rewrite();
    let source_identity = FileIdentity::from_metadata(&metadata);
    let copied_to = reconcile_appends(
        path,
        &mut source,
        source_identity,
        copied_to,
        &mut replacement,
        max_reconcile_wait,
    )
    .with_context(|| format!("preserve transcript appends for {}", path.display()))?;

    let old_inode_monitor = OldInodeMonitor::new(path)?;
    has_writable_open_handle(old_inode_monitor.path())
        .context("check transcript writer monitoring before replacement")?;
    let mut replacement_writer = fs::OpenOptions::new()
        .append(true)
        .open(replacement.path())
        .with_context(|| format!("open replacement transcript {} for append", path.display()))?;
    fs::set_permissions(replacement.path(), metadata.permissions())
        .with_context(|| format!("preserve transcript permissions for {}", path.display()))?;
    before_replace();
    let copied_to = reconcile_appends(
        path,
        &mut source,
        source_identity,
        copied_to,
        &mut replacement,
        max_reconcile_wait,
    )
    .with_context(|| format!("preserve final transcript appends for {}", path.display()))?;
    replacement
        .as_file()
        .sync_all()
        .with_context(|| format!("sync replacement transcript for {}", path.display()))?;
    let persisted_file = replacement
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("replace Claude Code transcript {}", path.display()))?;
    drop(persisted_file);
    after_replace();
    reconcile_in_flight_appends(
        old_inode_monitor.path(),
        &mut source,
        &mut replacement_writer,
        copied_to,
        &mut has_writable_open_handle,
        max_reconcile_wait,
    )
    .with_context(|| {
        format!(
            "preserve in-flight transcript appends for {}",
            path.display()
        )
    })?;

    let final_bytes = fs::metadata(path)
        .with_context(|| format!("inspect replaced transcript {}", path.display()))?
        .len();
    Ok(SlimResult {
        original_bytes,
        final_bytes,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

impl FileIdentity {
    fn from_metadata(metadata: &Metadata) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Self {
                device: metadata.dev(),
                inode: metadata.ino(),
            }
        }
        #[cfg(not(unix))]
        {
            let _ = metadata;
            Self {}
        }
    }

    fn matches_path(self, path: &Path) -> Result<bool> {
        let metadata = fs::metadata(path)
            .with_context(|| format!("inspect transcript path {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Ok(metadata.dev() == self.device && metadata.ino() == self.inode)
        }
        #[cfg(not(unix))]
        {
            let _ = (self, metadata);
            Ok(true)
        }
    }
}

fn reconcile_appends(
    path: &Path,
    source: &mut File,
    identity: FileIdentity,
    mut copied_to: u64,
    output: &mut NamedTempFile,
    max_wait: Duration,
) -> Result<u64> {
    if !identity.matches_path(path)? {
        bail!("transcript was replaced while it was being slimmed");
    }
    let deadline = Instant::now() + max_wait;
    loop {
        if Instant::now() >= deadline {
            bail!("timed out reconciling transcript appends before replacement");
        }
        let current_len = source
            .metadata()
            .context("inspect open transcript during append reconciliation")?
            .len();
        if current_len < copied_to {
            bail!("transcript shrank while it was being slimmed");
        }
        copied_to = copy_complete_lines(source, current_len, copied_to, output.as_file_mut())?;
        if !identity.matches_path(path)? {
            bail!("transcript was replaced while it was being slimmed");
        }
        let latest_len = source
            .metadata()
            .context("recheck open transcript during append reconciliation")?
            .len();
        if latest_len == current_len {
            return Ok(copied_to);
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn copy_complete_lines(
    source: &mut File,
    observed_len: u64,
    mut copied_to: u64,
    output: &mut File,
) -> Result<u64> {
    if observed_len <= copied_to {
        return Ok(copied_to);
    }
    source
        .seek(SeekFrom::Start(copied_to))
        .context("seek to appended transcript data")?;
    let available = observed_len - copied_to;
    let mut reader = BufReader::new((&mut *source).take(available));
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = reader
            .read_until(b'\n', &mut line)
            .context("read appended transcript line")?;
        if read == 0 || line.last() != Some(&b'\n') {
            return Ok(copied_to);
        }
        output
            .write_all(&line)
            .context("copy complete appended transcript line")?;
        copied_to += read as u64;
    }
}

struct OldInodeMonitor {
    path: TempPath,
}

impl OldInodeMonitor {
    fn new(transcript_path: &Path) -> Result<Self> {
        let parent = transcript_path
            .parent()
            .context("Claude Code transcript has no parent directory")?;
        let path = Builder::new()
            .prefix(".armyknife-transcript-slim-watch-")
            .tempfile_in(parent)
            .with_context(|| format!("create transcript monitor link in {}", parent.display()))?
            .into_temp_path();
        fs::remove_file(&path).context("prepare transcript monitor link path")?;
        fs::hard_link(transcript_path, &path)
            .with_context(|| format!("create monitor link for {}", transcript_path.display()))?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

fn reconcile_in_flight_appends<P>(
    old_inode_path: &Path,
    source: &mut File,
    replacement: &mut File,
    mut copied_to: u64,
    has_writable_open_handle: &mut P,
    max_wait: Duration,
) -> Result<()>
where
    P: FnMut(&Path) -> Result<bool>,
{
    const POLL_INTERVAL: Duration = Duration::from_millis(25);
    let deadline = Instant::now() + max_wait;
    let mut inspection_failed = false;

    loop {
        if Instant::now() >= deadline {
            bail!("timed out waiting for old transcript writers to close");
        }
        let current_len = source
            .metadata()
            .context("inspect original transcript for in-flight appends")?
            .len();
        if current_len > copied_to {
            copied_to = copy_complete_lines(source, current_len, copied_to, replacement)?;
            replacement
                .sync_data()
                .context("sync in-flight transcript append")?;
        }

        let has_writer = match has_writable_open_handle(old_inode_path) {
            Ok(has_writer) => {
                inspection_failed = false;
                has_writer
            }
            Err(error) => {
                if !inspection_failed {
                    eprintln!(
                        "Cannot inspect transcript writers after replacement; retrying: {error:#}"
                    );
                    inspection_failed = true;
                }
                if Instant::now() >= deadline {
                    bail!("timed out inspecting old transcript writers: {error:#}");
                }
                thread::sleep(POLL_INTERVAL);
                continue;
            }
        };

        if !has_writer {
            let final_len = source
                .metadata()
                .context("verify original transcript after writers close")?
                .len();
            if final_len == copied_to {
                replacement
                    .sync_all()
                    .context("sync final replacement transcript")?;
                return Ok(());
            }
        }
        if Instant::now() >= deadline {
            bail!("timed out waiting for old transcript writers to close");
        }
        thread::sleep(POLL_INTERVAL);
    }
}

#[cfg(all(not(test), target_os = "macos"))]
fn has_writable_open_handle(path: &Path) -> Result<bool> {
    let output = command::new("lsof")
        .args(["-nP", "-F0pfa"])
        .arg(path)
        .output()
        .context("run lsof to inspect transcript writers")?;
    if !output.status.success() {
        bail!("lsof could not inspect transcript writers");
    }
    Ok(lsof_output_has_writable_handle(&output.stdout))
}

#[cfg(all(not(test), target_os = "linux"))]
fn has_writable_open_handle(path: &Path) -> Result<bool> {
    use std::os::unix::fs::MetadataExt;

    let target = fs::metadata(path).context("inspect monitored transcript inode")?;
    let current_uid = fs::metadata("/proc/self")
        .context("inspect current process owner")?
        .uid();
    for process in fs::read_dir("/proc").context("list processes for transcript writer check")? {
        let process = process.context("read process entry")?;
        let Some(pid) = process
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let process_path = process.path();
        let process_metadata = match fs::metadata(&process_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("inspect process {pid} owner"));
            }
        };
        if process_metadata.uid() != current_uid {
            continue;
        }
        let fd_directory = process_path.join("fd");
        let file_descriptors = match fs::read_dir(&fd_directory) {
            Ok(file_descriptors) => file_descriptors,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("inspect file descriptors for process {pid}"));
            }
        };
        for file_descriptor in file_descriptors {
            let file_descriptor = file_descriptor.context("read process file descriptor")?;
            let descriptor_path = file_descriptor.path();
            let descriptor_metadata = match fs::metadata(&descriptor_path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("inspect process {pid} file descriptor {descriptor_path:?}")
                    });
                }
            };
            if descriptor_metadata.dev() != target.dev()
                || descriptor_metadata.ino() != target.ino()
            {
                continue;
            }
            let Some(descriptor_id) = file_descriptor.file_name().to_str().map(str::to_owned)
            else {
                continue;
            };
            let descriptor_info =
                match fs::read_to_string(process_path.join("fdinfo").join(descriptor_id)) {
                    Ok(descriptor_info) => descriptor_info,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => {
                        return Err(error)
                            .with_context(|| format!("read process {pid} descriptor flags"));
                    }
                };
            let flags = descriptor_info
                .lines()
                .find_map(|line| line.strip_prefix("flags:"))
                .context("process file descriptor has no flags field")?;
            let flags = u32::from_str_radix(flags.trim(), 8)
                .context("parse process file descriptor flags")?;
            if flags & (libc::O_ACCMODE as u32) != libc::O_RDONLY as u32 {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[cfg(all(not(test), not(any(target_os = "macos", target_os = "linux"))))]
fn has_writable_open_handle(_path: &Path) -> Result<bool> {
    bail!("transcript slimming requires macOS or Linux open-file inspection")
}

fn lsof_output_has_writable_handle(output: &[u8]) -> bool {
    output
        .split(|byte| *byte == 0)
        .any(|field| field.starts_with(b"aw") || field.starts_with(b"au"))
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::fs;
    use std::io::Write;
    use std::sync::Arc;
    use std::sync::Barrier;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::Duration;

    use indoc::indoc;
    use rstest::rstest;
    use serde_json::Value;
    use tempfile::tempdir;

    use super::{
        MAX_TRANSCRIPT_RECONCILE_WAIT, lsof_output_has_writable_handle, slim_transcript,
        slim_transcript_with,
    };

    #[test]
    fn slims_only_discardable_data_before_the_latest_compact_boundary() {
        let directory = tempdir().expect("temporary directory should be created");
        let transcript = directory.path().join("fixture-transcript.jsonl");
        fs::write(
            &transcript,
            indoc! {r#"
                {"type":"assistant","uuid":"fixture-read","message":{"content":[{"type":"tool_use","id":"fixture-tool","name":"Read","input":{"file_path":"/sandbox/sample.txt"}}]}}
                {"type":"user","uuid":"fixture-old-preserved","message":{"content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"fixture-old-base64"}}]}}
                {"type":"system","subtype":"compact_boundary","compactMetadata":{"preservedMessages":{"uuids":["fixture-old-preserved"]}}}
                {"type":"user","uuid":"fixture-image","parentUuid":"fixture-read","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-tool","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"fixture-base64"}}]}]},"toolUseResult":{"file":{"filePath":"/sandbox/sample.txt","base64":"fixture-base64","discarded":"fixture-payload"},"task_id":"fixture-task","taskId":"fixture-task-camel","background_task_id":"fixture-background","backgroundTaskId":"fixture-background-camel","shell_id":"fixture-shell","shellId":"fixture-shell-camel","agentId":"fixture-agent","status":"fixture-status","answers":{"fixture-question":"fixture-answer"},"questions":["fixture-question"],"extra":"fixture-payload"}}
                {"type":"attachment","uuid":"fixture-attachment","parentUuid":"fixture-image","attachment":{"type":"invoked_skills","content":["fixture-skill"]}}
                {"type":"api-request-blob","body":"fixture-payload"}
                {"type":"user","uuid":"fixture-preserved","message":{"content":"preserved fixture"}}
                {"type":"user","uuid":"fixture-preserved-image","message":{"content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"fixture-preserved-base64"}}]}}
                {"type":"system","subtype":"compact_boundary","compactMetadata":{"preservedMessages":{"uuids":["fixture-preserved","fixture-preserved-image"]}}}
                {"type":"assistant","uuid":"fixture-after","message":{"content":"after boundary"}}
            "#},
        )
        .expect("fixture transcript should be written");

        slim_transcript(&transcript).expect("transcript should be slimmed");

        assert_eq!(
            parse_jsonl(
                &fs::read_to_string(transcript).expect("slimmed transcript should be readable")
            ),
            parse_jsonl(indoc! {r#"
                {"type":"assistant","uuid":"fixture-read","message":{"content":[{"type":"tool_use","id":"fixture-tool","name":"Read","input":{"file_path":"/sandbox/sample.txt"}}]}}
                {"type":"user","uuid":"fixture-old-preserved","message":{"content":[{"type":"text","text":"[Image data omitted]"}]}}
                {"type":"system","subtype":"compact_boundary","compactMetadata":{"preservedMessages":{"uuids":["fixture-old-preserved"]}}}
                {"type":"user","uuid":"fixture-image","parentUuid":"fixture-read","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-tool","content":[{"type":"text","text":"[Image data omitted; source: /sandbox/sample.txt]"}]}]},"toolUseResult":{"task_id":"fixture-task","taskId":"fixture-task-camel","background_task_id":"fixture-background","backgroundTaskId":"fixture-background-camel","shell_id":"fixture-shell","shellId":"fixture-shell-camel","agentId":"fixture-agent","status":"fixture-status","answers":{"fixture-question":"fixture-answer"},"questions":["fixture-question"]}}
                {"type":"attachment","uuid":"fixture-attachment","parentUuid":"fixture-image","attachment":{"type":"invoked_skills"}}
                {"type":"user","uuid":"fixture-preserved","message":{"content":"preserved fixture"}}
                {"type":"user","uuid":"fixture-preserved-image","message":{"content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"fixture-preserved-base64"}}]}}
                {"type":"system","subtype":"compact_boundary","compactMetadata":{"preservedMessages":{"uuids":["fixture-preserved","fixture-preserved-image"]}}}
                {"type":"assistant","uuid":"fixture-after","message":{"content":"after boundary"}}
            "#})
        );
    }

    #[test]
    fn retains_required_attachments_and_post_boundary_usage() {
        let directory = tempdir().expect("temporary directory should be created");
        let transcript = directory.path().join("fixture-transcript.jsonl");
        fs::write(
            &transcript,
            indoc! {r#"
                {"type":"assistant","uuid":"fixture-read","message":{"content":[{"type":"tool_use","id":"fixture-read-tool","name":"Read","input":{"file_path":"/fixture/source.png"}}]}}
                {"type":"attachment","uuid":"fixture-skill-listing","parentUuid":"fixture-read","attachment":{"type":"skill_listing","content":["fixture-skill"]}}
                {"type":"attachment","uuid":"fixture-queued-command","parentUuid":"fixture-read","attachment":{"type":"queued_command","prompt":"data:image/png;base64,fixture-payload"}}
                {"type":"user","uuid":"fixture-tool-result-image","message":{"content":"fixture answer"},"toolUseResult":{"file":{"filePath":"/fixture/source.png","base64":"fixture-file-payload"},"answers":{"image":"data:image/png;base64,fixture-answer-image"}}}
                {"type":"attachment","uuid":"fixture-task-notification","parentUuid":"fixture-read","attachment":{"type":"task-notification","content":"fixture task"}}
                {"type":"attachment","uuid":"fixture-invoked-skills","parentUuid":"fixture-read","attachment":{"type":"invoked_skills","content":["fixture-skill"]}}
                {"type":"attachment","uuid":"fixture-deferred-tools","parentUuid":"fixture-read","attachment":{"type":"deferred_tools_delta","content":"fixture tools"}}
                {"type":"attachment","uuid":"fixture-prompt-snapshot","parentUuid":"fixture-read","attachment":{"type":"prompt_snapshot","content":"fixture prompt"}}
                {"type":"attachment","uuid":"fixture-instructions","parentUuid":"fixture-read","attachment":{"type":"instructions","content":"fixture instructions"}}
                {"type":"attachment","uuid":"fixture-hook-success","parentUuid":"fixture-read","attachment":{"type":"hook_success","content":"fixture hook"}}
                {"type":"api-request","body":"fixture payload"}
                {"type":"api-request-blob","body":"fixture payload"}
                {"type":"api-request-shape","body":"fixture payload"}
                {"type":"system","subtype":"compact_boundary","compactMetadata":{"preservedMessages":{"uuids":[]}}}
                {"type":"custom-title","customTitle":"Fixture title"}
                {"type":"assistant","uuid":"fixture-after","usage":{"input_tokens":1}}
            "#},
        )
        .expect("fixture transcript should be written");

        slim_transcript(&transcript).expect("transcript should be slimmed");

        assert_eq!(
            parse_jsonl(
                &fs::read_to_string(transcript).expect("slimmed transcript should be readable")
            ),
            parse_jsonl(indoc! {r#"
                {"type":"assistant","uuid":"fixture-read","message":{"content":[{"type":"tool_use","id":"fixture-read-tool","name":"Read","input":{"file_path":"/fixture/source.png"}}]}}
                {"type":"attachment","uuid":"fixture-skill-listing","parentUuid":"fixture-read","attachment":{"type":"skill_listing","content":["fixture-skill"]}}
                {"type":"attachment","uuid":"fixture-queued-command","parentUuid":"fixture-read","attachment":{"type":"queued_command","prompt":"[Image data omitted]"}}
                {"type":"user","uuid":"fixture-tool-result-image","message":{"content":"fixture answer"},"toolUseResult":{"answers":{"image":"[Image data omitted]"}}}
                {"type":"attachment","uuid":"fixture-task-notification","parentUuid":"fixture-read","attachment":{"type":"task-notification","content":"fixture task"}}
                {"type":"attachment","uuid":"fixture-invoked-skills","parentUuid":"fixture-read","attachment":{"type":"invoked_skills"}}
                {"type":"attachment","uuid":"fixture-deferred-tools","parentUuid":"fixture-read","attachment":{"type":"deferred_tools_delta"}}
                {"type":"attachment","uuid":"fixture-prompt-snapshot","parentUuid":"fixture-read","attachment":{"type":"prompt_snapshot"}}
                {"type":"attachment","uuid":"fixture-instructions","parentUuid":"fixture-read","attachment":{"type":"instructions"}}
                {"type":"attachment","uuid":"fixture-hook-success","parentUuid":"fixture-read","attachment":{"type":"hook_success"}}
                {"type":"system","subtype":"compact_boundary","compactMetadata":{"preservedMessages":{"uuids":[]}}}
                {"type":"custom-title","customTitle":"Fixture title"}
                {"type":"assistant","uuid":"fixture-after","usage":{"input_tokens":1}}
            "#})
        );
    }

    #[test]
    fn preserves_appends_during_rewrite_and_across_replacement() {
        let directory = tempdir().expect("temporary directory should be created");
        let transcript = directory.path().join("fixture-transcript.jsonl");
        fs::write(
            &transcript,
            indoc! {r#"
                {"type":"user","uuid":"fixture-old","message":{"content":"fixture old"}}
                {"type":"system","subtype":"compact_boundary","compactMetadata":{"preservedMessages":{"uuids":[]}}}
            "#},
        )
        .expect("fixture transcript should be written");
        let old_writer = fs::OpenOptions::new()
            .append(true)
            .open(&transcript)
            .expect("original transcript should be appendable");
        let writer_open = Arc::new(AtomicBool::new(true));
        let callback_writer_open = Arc::clone(&writer_open);
        let replacement_is_live = Arc::new(AtomicBool::new(false));
        let callback_replacement_is_live = Arc::clone(&replacement_is_live);
        let split_row_barrier = Arc::new(Barrier::new(2));
        let writer_split_row_barrier = Arc::clone(&split_row_barrier);
        let mut inserted_new_path_row = false;
        let mut late_writer_thread = None;

        slim_transcript_with(
            &transcript,
            || {
                let mut original = fs::OpenOptions::new()
                    .append(true)
                    .open(&transcript)
                    .expect("original transcript should be appendable during rewrite");
                original
                    .write_all(b"{\"type\":\"user\",\"uuid\":\"fixture-during\",\"message\":{\"content\":\"fixture during\"}}\n")
                    .expect("append during rewrite should succeed");
            },
            || {
                let mut original = fs::OpenOptions::new()
                    .append(true)
                    .open(&transcript)
                    .expect("original transcript should be appendable before replacement");
                original
                    .write_all(b"{\"type\":\"user\",\"uuid\":\"fixture-before-replace\",\"message\":{\"content\":\"fixture before replace\"}}\n")
                    .expect("append before replacement should succeed");
            },
            || {
                callback_replacement_is_live.store(true, Ordering::SeqCst);
                late_writer_thread = Some(thread::spawn(move || {
                    thread::sleep(Duration::from_millis(100));
                    let mut old_writer = old_writer;
                    old_writer.write_all(b"{\"type\":\"user\",\"uuid\":\"fixture-old-fd-after-replace\"")?;
                    writer_split_row_barrier.wait();
                    writer_split_row_barrier.wait();
                    let result = old_writer.write_all(b",\"message\":{\"content\":\"fixture old fd after replace\"}}\n");
                    drop(old_writer);
                    callback_writer_open.store(false, Ordering::SeqCst);
                    result
                }));
            },
            |_| {
                if replacement_is_live.load(Ordering::SeqCst) && !inserted_new_path_row {
                    split_row_barrier.wait();
                    let mut replacement = fs::OpenOptions::new()
                        .append(true)
                        .open(&transcript)
                        .expect("replacement transcript should be appendable");
                    replacement
                        .write_all(b"{\"type\":\"user\",\"uuid\":\"fixture-new-path-after-replace\",\"message\":{\"content\":\"fixture new path after replace\"}}\n")
                        .expect("append through the new path should succeed");
                    inserted_new_path_row = true;
                    split_row_barrier.wait();
                }
                Ok(writer_open.load(Ordering::SeqCst))
            },
            Duration::from_secs(1),
        )
        .expect("transcript should be slimmed");
        late_writer_thread
            .expect("late writer thread should be created")
            .join()
            .expect("late writer thread should finish")
            .expect("append through the old file handle should succeed");

        assert_eq!(
            parse_jsonl(&fs::read_to_string(&transcript).expect("transcript should be readable")),
            parse_jsonl(indoc! {r#"
                {"type":"user","uuid":"fixture-old","message":{"content":"fixture old"}}
                {"type":"system","subtype":"compact_boundary","compactMetadata":{"preservedMessages":{"uuids":[]}}}
                {"type":"user","uuid":"fixture-during","message":{"content":"fixture during"}}
                {"type":"user","uuid":"fixture-before-replace","message":{"content":"fixture before replace"}}
                {"type":"user","uuid":"fixture-new-path-after-replace","message":{"content":"fixture new path after replace"}}
                {"type":"user","uuid":"fixture-old-fd-after-replace","message":{"content":"fixture old fd after replace"}}
            "#})
        );
    }

    #[test]
    fn stops_after_writer_inspection_timeout_and_cleans_monitor_link() {
        let directory = tempdir().expect("temporary directory should be created");
        let transcript = directory.path().join("fixture-transcript.jsonl");
        fs::write(
            &transcript,
            indoc! {r#"
                {"type":"user","uuid":"fixture-old","message":{"content":"fixture old"}}
                {"type":"system","subtype":"compact_boundary","compactMetadata":{"preservedMessages":{"uuids":[]}}}
            "#},
        )
        .expect("fixture transcript should be written");
        let monitor_path = RefCell::new(None);
        let inspection_count = Cell::new(0);

        let result = slim_transcript_with(
            &transcript,
            || {},
            || {},
            || {},
            |path| {
                *monitor_path.borrow_mut() = Some(path.to_path_buf());
                inspection_count.set(inspection_count.get() + 1);
                if inspection_count.get() == 1 {
                    Ok(false)
                } else {
                    Err(anyhow::anyhow!("fixture inspection failure"))
                }
            },
            Duration::from_millis(50),
        );
        let monitor_path = monitor_path
            .into_inner()
            .expect("monitor path should be inspected");

        assert_eq!(
            (
                result.is_err(),
                inspection_count.get() > 1,
                monitor_path.exists(),
            ),
            (true, true, false),
        );
    }

    #[cfg(unix)]
    #[test]
    fn preserves_read_only_permissions_after_replacement() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempdir().expect("temporary directory should be created");
        let transcript = directory.path().join("fixture-transcript.jsonl");
        fs::write(
            &transcript,
            indoc! {r#"
                {"type":"user","uuid":"fixture-old","message":{"content":"fixture old"}}
                {"type":"system","subtype":"compact_boundary","compactMetadata":{"preservedMessages":{"uuids":[]}}}
            "#},
        )
        .expect("fixture transcript should be written");
        fs::set_permissions(&transcript, fs::Permissions::from_mode(0o444))
            .expect("fixture transcript should become read-only");

        slim_transcript_with(
            &transcript,
            || {},
            || {},
            || {},
            |_| Ok(false),
            MAX_TRANSCRIPT_RECONCILE_WAIT,
        )
        .expect("read-only transcript should be slimmed");

        assert_eq!(
            (
                fs::read_to_string(&transcript).expect("slimmed transcript should be readable"),
                fs::metadata(&transcript)
                    .expect("slimmed transcript metadata should be readable")
                    .permissions()
                    .mode()
                    & 0o777,
            ),
            (
                indoc! {r#"
                    {"type":"user","uuid":"fixture-old","message":{"content":"fixture old"}}
                    {"type":"system","subtype":"compact_boundary","compactMetadata":{"preservedMessages":{"uuids":[]}}}
                "#}
                .to_owned(),
                0o444,
            ),
        );
    }

    #[test]
    fn reduces_non_object_tool_use_results_without_changing_null() {
        let directory = tempdir().expect("temporary directory should be created");
        let transcript = directory.path().join("fixture-transcript.jsonl");
        fs::write(
            &transcript,
            indoc! {r#"
                {"type":"user","uuid":"fixture-string-result","toolUseResult":"fixture-payload"}
                {"type":"user","uuid":"fixture-array-result","toolUseResult":["fixture-payload"]}
                {"type":"user","uuid":"fixture-null-result","toolUseResult":null}
                {"type":"system","subtype":"compact_boundary","compactMetadata":{"preservedMessages":{"uuids":[]}}}
            "#},
        )
        .expect("fixture transcript should be written");

        slim_transcript(&transcript).expect("transcript should be slimmed");

        assert_eq!(
            parse_jsonl(
                &fs::read_to_string(transcript).expect("slimmed transcript should be readable")
            ),
            parse_jsonl(indoc! {r#"
                {"type":"user","uuid":"fixture-string-result","toolUseResult":{}}
                {"type":"user","uuid":"fixture-array-result","toolUseResult":{}}
                {"type":"user","uuid":"fixture-null-result","toolUseResult":null}
                {"type":"system","subtype":"compact_boundary","compactMetadata":{"preservedMessages":{"uuids":[]}}}
            "#})
        );
    }

    #[test]
    fn leaves_transcript_without_a_compact_boundary_unchanged() {
        let directory = tempdir().expect("temporary directory should be created");
        let transcript = directory.path().join("fixture-transcript.jsonl");
        let original = "{\"type\":\"api-request\",\"body\":\"fixture\"}\n";
        fs::write(&transcript, original).expect("fixture transcript should be written");

        slim_transcript(&transcript).expect("transcript should be inspected");

        assert_eq!(
            fs::read_to_string(transcript).expect("transcript should be readable"),
            original
        );
    }

    #[rstest]
    #[case::single_path_component("fixture-session", true)]
    #[case::parent_path("../fixture-session", false)]
    fn validates_session_ids_as_single_path_components(
        #[case] session_id: &str,
        #[case] expected_valid: bool,
    ) {
        assert_eq!(
            super::validate_session_id(session_id).is_ok(),
            expected_valid
        );
    }

    #[rstest]
    #[case::read_only(b"p123\0f4\0ar\0", false)]
    #[case::write_only(b"p123\0f4\0aw\0", true)]
    #[case::read_and_write(b"p123\0f4\0au\0", true)]
    fn detects_writable_open_handles_in_lsof_output(#[case] output: &[u8], #[case] expected: bool) {
        assert_eq!(lsof_output_has_writable_handle(output), expected);
    }

    fn parse_jsonl(source: &str) -> Vec<Value> {
        source
            .lines()
            .map(|line| serde_json::from_str(line).expect("fixture row should be valid JSON"))
            .collect()
    }
}
