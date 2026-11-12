use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use anyhow::Result;
use axum::body::Body;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS};
use axum::response::{IntoResponse, Response};

use super::{BackgroundTaskKind, MessagesRouteState};
use crate::commands::agent::types::{Engine, Session};

const MAX_OUTPUT_BYTES: u64 = 64 * 1024;
const MAX_OUTPUT_LINES: usize = 20;
const MAX_CACHED_TRANSCRIPTS: usize = 128;
const OUTPUT_UNAVAILABLE: &str = "Output is unavailable for this task.";
static OUTPUT_PATH_CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedOutputPaths>>> = OnceLock::new();

#[derive(Clone, Copy, PartialEq, Eq)]
struct TranscriptFingerprint {
    length: u64,
    modified: SystemTime,
}

struct CachedOutputPaths {
    fingerprint: TranscriptFingerprint,
    paths: HashMap<String, PathBuf>,
}

pub(super) async fn get_background_task_output(
    State((load_session, _, _)): State<MessagesRouteState>,
    AxumPath((session_id, task_id)): AxumPath<(String, String)>,
) -> Response {
    let output = match tokio::task::spawn_blocking(move || -> Result<Option<String>> {
        let Some(session) = load_session(&session_id)? else {
            return Ok(None);
        };
        let Some(path) = output_path_for_task(&session, &task_id)? else {
            return Ok(None);
        };
        Ok(read_output_tail(&path)?)
    })
    .await
    {
        Ok(Ok(Some(output))) => output,
        Ok(Ok(None)) => OUTPUT_UNAVAILABLE.to_owned(),
        Ok(Err(error)) => {
            tracing::warn!(%error, "failed to read background task output");
            OUTPUT_UNAVAILABLE.to_owned()
        }
        Err(error) => {
            tracing::error!(%error, "background task output worker failed");
            OUTPUT_UNAVAILABLE.to_owned()
        }
    };
    (
        StatusCode::OK,
        [
            (CONTENT_TYPE, "text/plain; charset=utf-8"),
            (X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (CACHE_CONTROL, "no-store"),
        ],
        Body::from(output),
    )
        .into_response()
}

fn output_path_for_task(session: &Session, task_id: &str) -> Result<Option<PathBuf>> {
    if session.engine != Engine::Claude {
        return Ok(None);
    }
    let Some(transcript_path) = session.transcript_path.as_deref() else {
        return Ok(None);
    };
    let fingerprint = match fs::metadata(transcript_path) {
        Ok(metadata) if metadata.is_file() => {
            metadata
                .modified()
                .ok()
                .map(|modified| TranscriptFingerprint {
                    length: metadata.len(),
                    modified,
                })
        }
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if let Some(fingerprint) = fingerprint
        && let Ok(cache) = output_path_cache().lock()
        && let Some(cached) = cache.get(transcript_path)
        && cached.fingerprint == fingerprint
    {
        return Ok(cached.paths.get(task_id).cloned());
    }

    let Some(reader) = super::open_seekable_transcript(transcript_path)? else {
        return Ok(None);
    };
    let decisions = Vec::new();
    let queued_prompts = Vec::new();
    let peer_out_messages = Vec::new();
    let proposals = Vec::new();
    let (_, tasks, _) =
        super::parser::claude::parse_with_queued_prompts_and_peer_out_messages_and_proposals(
            reader,
            chrono::Utc::now(),
            super::parser::claude::ParserAdditions {
                parent: None,
                pending_tool: None,
                user_marker: None,
                decisions: &decisions,
                queued_prompts: &queued_prompts,
                peer_out_messages: &peer_out_messages,
                proposals: &proposals,
                todos: &[],
            },
        )?;
    let paths = tasks
        .into_iter()
        .filter(|task| task.kind == BackgroundTaskKind::BackgroundTask)
        .filter_map(|task| task.output_path.map(|path| (task.id, path)))
        .collect::<HashMap<_, _>>();
    let output_path = paths.get(task_id).cloned();
    if let Some(fingerprint) = fingerprint
        && let Ok(mut cache) = output_path_cache().lock()
    {
        if cache.len() >= MAX_CACHED_TRANSCRIPTS
            && let Some(path) = cache.keys().next().cloned()
        {
            cache.remove(&path);
        }
        cache.insert(
            transcript_path.to_path_buf(),
            CachedOutputPaths { fingerprint, paths },
        );
    }
    Ok(output_path)
}

fn output_path_cache() -> &'static Mutex<HashMap<PathBuf, CachedOutputPaths>> {
    OUTPUT_PATH_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn read_output_tail(path: &Path) -> std::io::Result<Option<String>> {
    if !path.is_absolute() {
        return Ok(None);
    }
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => metadata,
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    let start = metadata.len().saturating_sub(MAX_OUTPUT_BYTES);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::with_capacity(usize::try_from(MAX_OUTPUT_BYTES).unwrap_or_default());
    file.take(MAX_OUTPUT_BYTES).read_to_end(&mut bytes)?;

    let mut output = String::from_utf8_lossy(&bytes).into_owned();
    if start > 0
        && let Some(line_end) = output.find('\n')
    {
        output.drain(..=line_end);
    }
    let lines = output.split_inclusive('\n').collect::<Vec<_>>();
    let first_line = lines.len().saturating_sub(MAX_OUTPUT_LINES);
    Ok(Some(lines[first_line..].concat()))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{MAX_OUTPUT_BYTES, read_output_tail};

    #[test]
    fn reads_only_the_last_twenty_lines() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("output.log");
        let output = (1..=25)
            .map(|line| format!("line {line}\n"))
            .collect::<String>();
        fs::write(&path, output).unwrap();

        assert_eq!(
            read_output_tail(&path).unwrap(),
            Some((6..=25).map(|line| format!("line {line}\n")).collect()),
        );
    }

    #[test]
    fn caps_the_read_size_for_large_output_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("output.log");
        let output = "x".repeat(usize::try_from(MAX_OUTPUT_BYTES * 2).unwrap());
        fs::write(&path, &output).unwrap();

        let expected =
            output[output.len() - usize::try_from(MAX_OUTPUT_BYTES).unwrap()..].to_owned();
        assert_eq!(read_output_tail(&path).unwrap(), Some(expected));
    }
}
