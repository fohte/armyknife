use std::collections::{HashMap, HashSet};
use std::fs::{self, File, Metadata};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use tempfile::NamedTempFile;

use super::super::claude_sessions::session_jsonl_path;

mod transform;

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
    read_paths_by_record: HashMap<String, String>,
}

fn slim_transcript(path: &Path) -> Result<SlimResult> {
    slim_transcript_with(path, || {}, || {}, || {})
}

fn slim_transcript_with<F, G, H>(
    path: &Path,
    during_rewrite: F,
    before_replace: G,
    after_replace: H,
) -> Result<SlimResult>
where
    F: FnOnce(),
    G: FnOnce(),
    H: FnOnce(),
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
    transform::write_slimmed_prefix(
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
        original_bytes,
        &mut replacement,
    )
    .with_context(|| format!("preserve transcript appends for {}", path.display()))?;

    fs::set_permissions(replacement.path(), metadata.permissions())
        .with_context(|| format!("preserve transcript permissions for {}", path.display()))?;
    before_replace();
    let copied_to = reconcile_appends(
        path,
        &mut source,
        source_identity,
        copied_to,
        &mut replacement,
    )
    .with_context(|| format!("preserve final transcript appends for {}", path.display()))?;
    replacement
        .as_file()
        .sync_all()
        .with_context(|| format!("sync replacement transcript for {}", path.display()))?;
    replacement
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("replace Claude Code transcript {}", path.display()))?;
    after_replace();
    reconcile_in_flight_appends(path, &mut source, copied_to).with_context(|| {
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
) -> Result<u64> {
    if !identity.matches_path(path)? {
        bail!("transcript was replaced while it was being slimmed");
    }
    loop {
        let current_len = source
            .metadata()
            .context("inspect open transcript during append reconciliation")?
            .len();
        if current_len < copied_to {
            bail!("transcript shrank while it was being slimmed");
        }
        if current_len > copied_to {
            source
                .seek(SeekFrom::Start(copied_to))
                .context("seek to appended transcript data")?;
            let mut appended = (&mut *source).take(current_len - copied_to);
            let copied =
                std::io::copy(&mut appended, output).context("copy appended transcript data")?;
            copied_to += copied;
        }
        if !identity.matches_path(path)? {
            bail!("transcript was replaced while it was being slimmed");
        }
        let latest_len = source
            .metadata()
            .context("recheck open transcript during append reconciliation")?
            .len();
        if latest_len == copied_to {
            return Ok(copied_to);
        }
    }
}

fn reconcile_in_flight_appends(path: &Path, source: &mut File, mut copied_to: u64) -> Result<()> {
    let mut replacement = fs::OpenOptions::new()
        .append(true)
        .open(path)
        .with_context(|| format!("open replacement transcript {} for append", path.display()))?;
    let mut observed_len = source
        .metadata()
        .context("inspect original transcript after replacement")?
        .len();
    let mut stable_since = Instant::now();

    loop {
        let current_len = source
            .metadata()
            .context("inspect original transcript for in-flight appends")?
            .len();
        if current_len > copied_to {
            source
                .seek(SeekFrom::Start(copied_to))
                .context("seek to in-flight transcript append")?;
            let mut appended = (&mut *source).take(current_len - copied_to);
            copied_to += std::io::copy(&mut appended, &mut replacement)
                .context("copy in-flight transcript append")?;
            replacement
                .sync_data()
                .context("sync in-flight transcript append")?;
            observed_len = source
                .metadata()
                .context("recheck original transcript after in-flight append")?
                .len();
            stable_since = Instant::now();
        } else if current_len != observed_len {
            observed_len = current_len;
            stable_since = Instant::now();
        } else if stable_since.elapsed() >= Duration::from_millis(25) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;

    use indoc::indoc;
    use serde_json::Value;
    use tempfile::tempdir;

    use super::{slim_transcript, slim_transcript_with};

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
                {"type":"system","subtype":"compact_boundary","compactMetadata":{"preservedMessages":{"uuids":["fixture-preserved"]}}}
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
                {"type":"system","subtype":"compact_boundary","compactMetadata":{"preservedMessages":{"uuids":["fixture-preserved"]}}}
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
                {"type":"attachment","uuid":"fixture-queued-command","parentUuid":"fixture-read","attachment":{"type":"queued_command","prompt":"[Image data omitted; source: /fixture/source.png]"}}
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
        let old_writer = std::cell::RefCell::new(Some(
            fs::OpenOptions::new()
                .append(true)
                .open(&transcript)
                .expect("original transcript should be appendable"),
        ));

        slim_transcript_with(&transcript, || {
            let mut original = fs::OpenOptions::new()
                .append(true)
                .open(&transcript)
                .expect("original transcript should be appendable during rewrite");
            original
                .write_all(b"{\"type\":\"user\",\"uuid\":\"fixture-during\",\"message\":{\"content\":\"fixture during\"}}\n")
                .expect("append during rewrite should succeed");
        }, || {
            let mut original = fs::OpenOptions::new()
                .append(true)
                .open(&transcript)
                .expect("original transcript should be appendable before replacement");
            original
                .write_all(b"{\"type\":\"user\",\"uuid\":\"fixture-before-replace\",\"message\":{\"content\":\"fixture before replace\"}}\n")
                .expect("append before replacement should succeed");
        }, || {
            let mut replacement = fs::OpenOptions::new()
                .append(true)
                .open(&transcript)
                .expect("replacement transcript should be appendable");
            replacement
                .write_all(b"{\"type\":\"user\",\"uuid\":\"fixture-new-path-after-replace\",\"message\":{\"content\":\"fixture new path after replace\"}}\n")
                .expect("append through the new path should succeed");
            old_writer
                .borrow_mut()
                .take()
                .expect("pre-replacement file handle should still be open")
                .write_all(b"{\"type\":\"user\",\"uuid\":\"fixture-old-fd-after-replace\",\"message\":{\"content\":\"fixture old fd after replace\"}}\n")
                .expect("append through the old file handle should succeed");
        })
        .expect("transcript should be slimmed");

        assert_eq!(
            fs::read_to_string(&transcript).expect("transcript should be readable"),
            indoc! {r#"
                {"type":"user","uuid":"fixture-old","message":{"content":"fixture old"}}
                {"type":"system","subtype":"compact_boundary","compactMetadata":{"preservedMessages":{"uuids":[]}}}
                {"type":"user","uuid":"fixture-during","message":{"content":"fixture during"}}
                {"type":"user","uuid":"fixture-before-replace","message":{"content":"fixture before replace"}}
                {"type":"user","uuid":"fixture-new-path-after-replace","message":{"content":"fixture new path after replace"}}
                {"type":"user","uuid":"fixture-old-fd-after-replace","message":{"content":"fixture old fd after replace"}}
            "#}
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

    #[test]
    fn validates_session_ids_as_single_path_components() {
        assert!(super::validate_session_id("fixture-session").is_ok());
        assert!(super::validate_session_id("../fixture-session").is_err());
    }

    fn parse_jsonl(source: &str) -> Vec<Value> {
        source
            .lines()
            .map(|line| serde_json::from_str(line).expect("fixture row should be valid JSON"))
            .collect()
    }
}
