use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::future;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use anyhow::Result;
use axum::body::Body;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, HeaderValue, X_CONTENT_TYPE_OPTIONS};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use futures_util::stream::{self, Stream};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

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

pub(super) async fn get_background_task_output_events(
    State((load_session, _, loaders)): State<MessagesRouteState>,
    AxumPath((session_id, task_id)): AxumPath<(String, String)>,
) -> Response {
    let create_event_stream = loaders.background_task_output_event_stream.clone();
    let (output_path, output_events) = match tokio::task::spawn_blocking(
        move || -> Result<Option<(PathBuf, super::routes::BackgroundTaskOutputEventStream)>> {
            let Some(session) = load_session(&session_id)? else {
                return Ok(None);
            };
            let Some(output_path) = output_path_for_task(&session, &task_id, &loaders)? else {
                return Ok(None);
            };
            let events = create_event_stream(output_path.clone())?;
            Ok(Some((output_path, events)))
        },
    )
    .await
    {
        Ok(Ok(Some((path, events)))) => (path, events),
        Ok(Ok(None)) => return output_unavailable_event_response(),
        Ok(Err(error)) => {
            tracing::warn!(%error, "failed to prepare background task output event stream");
            return output_unavailable_response(StatusCode::INTERNAL_SERVER_ERROR);
        }
        Err(error) => {
            tracing::error!(%error, "background task output event stream worker failed");
            return output_unavailable_response(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    let changes = output_events.map(|_| ());
    let events = output_updates(output_path, changes)
        .map(|output| Ok::<Event, std::convert::Infallible>(output_event(output)));
    output_event_stream_response(events)
}

fn output_unavailable_event_response() -> Response {
    let events = stream::once(future::ready(Ok::<_, std::convert::Infallible>(
        output_event(OUTPUT_UNAVAILABLE.to_owned()),
    )))
    .chain(stream::pending());
    output_event_stream_response(events)
}

fn output_event(output: String) -> Event {
    Event::default().data(serde_json::json!({"output": output}).to_string())
}

fn output_event_stream_response(
    events: impl Stream<Item = Result<Event, std::convert::Infallible>> + Send + 'static,
) -> Response {
    let mut response = Sse::new(events)
        .keep_alive(KeepAlive::default())
        .into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    response
}

fn output_unavailable_response(status: StatusCode) -> Response {
    (
        status,
        [
            (CONTENT_TYPE, "text/plain; charset=utf-8"),
            (X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (CACHE_CONTROL, "no-store"),
        ],
        Body::from(OUTPUT_UNAVAILABLE),
    )
        .into_response()
}

type OutputEventCallback = Box<dyn FnMut(notify::Result<notify::Event>) + Send + 'static>;

pub(super) fn watch_output_events(
    path: PathBuf,
) -> Result<impl Stream<Item = notify::Result<notify::Event>> + Send> {
    watch_output_events_with(path, move |parent, callback| {
        let mut watcher: RecommendedWatcher = notify::recommended_watcher(callback)?;
        watcher.watch(&parent, RecursiveMode::NonRecursive)?;
        Ok(watcher)
    })
}

fn watch_output_events_with<W>(
    path: PathBuf,
    register: impl FnOnce(PathBuf, OutputEventCallback) -> Result<W> + Send + 'static,
) -> Result<impl Stream<Item = notify::Result<notify::Event>> + Send>
where
    W: Send + 'static,
{
    let watched_path = canonical_output_path(&path)?;
    let parent = watched_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("background task output path has no parent"))?
        .to_path_buf();
    let (sender, receiver) = mpsc::channel(1);
    let callback: OutputEventCallback = Box::new(move |event| {
        forward_matching_output_event(&sender, &watched_path, event);
    });
    let watcher = register(parent, callback)?;

    Ok(stream::unfold(
        (receiver, watcher),
        |(mut receiver, _watcher)| async move {
            let event = receiver.recv().await?;
            Some((event, (receiver, _watcher)))
        },
    ))
}

fn forward_matching_output_event(
    sender: &mpsc::Sender<notify::Result<notify::Event>>,
    watched_path: &Path,
    event: notify::Result<notify::Event>,
) {
    match &event {
        Ok(event)
            if !event
                .paths
                .iter()
                .any(|event_path| event_path == watched_path) =>
        {
            return;
        }
        Ok(event) if event.kind.is_access() => return,
        Err(error) => {
            tracing::warn!(%error, path = %watched_path.display(), "background task output watcher failed");
        }
        _ => {}
    }

    // Matching events can coalesce because each signal rereads the full output tail.
    let _ = sender.try_send(event);
}

fn canonical_output_path(path: &Path) -> Result<PathBuf> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("background task output path has no parent"))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("background task output path has no file name"))?;
    let canonical_parent = fs::canonicalize(parent)?;
    let canonical_path = canonical_parent.join(file_name);

    match fs::symlink_metadata(&canonical_path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(anyhow::anyhow!("background task output path is a symlink"))
        }
        Ok(metadata) if metadata.is_file() => Ok(fs::canonicalize(canonical_path)?),
        Ok(_) => Err(anyhow::anyhow!("background task output path is not a file")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(canonical_path),
        Err(error) => Err(error.into()),
    }
}

fn output_updates(
    path: PathBuf,
    changes: impl Stream<Item = ()> + Send + 'static,
) -> impl Stream<Item = String> + Send {
    let initial_path = path.clone();
    stream::once(output_for_event(initial_path))
        .chain(changes.then(move |_| output_for_event(path.clone())))
}

async fn output_for_event(path: PathBuf) -> String {
    let log_path = path.clone();
    match tokio::task::spawn_blocking(move || read_output_tail(&path)).await {
        Ok(Ok(Some(output))) => output,
        Ok(Ok(None)) => OUTPUT_UNAVAILABLE.to_owned(),
        Ok(Err(error)) => {
            tracing::warn!(%error, path = %log_path.display(), "failed to read background task output update");
            OUTPUT_UNAVAILABLE.to_owned()
        }
        Err(error) => {
            tracing::error!(%error, path = %log_path.display(), "background task output reader failed");
            OUTPUT_UNAVAILABLE.to_owned()
        }
    }
}

fn output_path_for_task(
    session: &Session,
    task_id: &str,
    loaders: &super::RouteLoaders,
) -> Result<Option<PathBuf>> {
    if let Some(path) = loaders
        .background_tasks(&session.session_id)?
        .into_iter()
        .find(|task| task.id == task_id)
        .and_then(|task| task.output_path)
    {
        return Ok(Some(path));
    }
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
                session_id: &session.session_id,
                parent: None,
                pending_tool: None,
                user_marker: None,
                decisions: &decisions,
                queued_prompts: &queued_prompts,
                peer_out_messages: &peer_out_messages,
                proposals: &proposals,
                todos: &[],
                additional_cards: &[],
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
    use std::sync::{Arc, Mutex};

    use futures_util::StreamExt;
    use notify::event::{AccessKind, AccessMode, ModifyKind};
    use notify::{Event, EventKind};
    use rstest::rstest;

    use super::{
        MAX_OUTPUT_BYTES, canonical_output_path, forward_matching_output_event, output_updates,
        read_output_tail, watch_output_events_with,
    };

    #[tokio::test]
    async fn background_task_output_unrelated_events_do_not_suppress_matching_updates() {
        let directory = tempfile::tempdir().unwrap();
        let output_path = directory.path().join("current.output");
        fs::write(&output_path, "initial output").unwrap();
        let callback = Arc::new(Mutex::new(None));
        let registered_callback = Arc::clone(&callback);
        let events = watch_output_events_with(output_path.clone(), move |_, handler| {
            *registered_callback.lock().unwrap() = Some(handler);
            Ok(())
        })
        .unwrap();
        let mut outputs = Box::pin(output_updates(output_path.clone(), events.map(|_| ())));
        let initial_output = outputs.next().await.unwrap();

        fs::write(&output_path, "updated output").unwrap();
        let mut callback = callback.lock().unwrap().take().unwrap();
        callback(Ok(
            Event::new(EventKind::Any).add_path(output_path.with_file_name("other.output"))
        ));
        callback(Ok(Event::new(EventKind::Any).add_path(output_path)));
        let updated_output = outputs.next().await.unwrap();

        assert_eq!(
            (initial_output, updated_output),
            ("initial output".to_owned(), "updated output".to_owned()),
        );
    }

    #[rstest]
    #[case::open(AccessKind::Open(AccessMode::Read))]
    #[case::close(AccessKind::Close(AccessMode::Read))]
    fn background_task_output_access_events_do_not_trigger_rereads(
        #[case] access_kind: AccessKind,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let watched_path = directory.path().join("output.log");
        fs::write(&watched_path, "initial output").unwrap();
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);

        forward_matching_output_event(
            &sender,
            &watched_path,
            Ok(Event::new(EventKind::Access(access_kind)).add_path(watched_path.clone())),
        );
        let access_was_forwarded = receiver.try_recv().is_ok();

        let modification = EventKind::Modify(ModifyKind::Any);
        forward_matching_output_event(
            &sender,
            &watched_path,
            Ok(Event::new(modification).add_path(watched_path.clone())),
        );
        let forwarded_modification = receiver
            .try_recv()
            .ok()
            .and_then(Result::ok)
            .map(|event| (event.kind, event.paths));

        assert_eq!(
            (access_was_forwarded, forwarded_modification),
            (false, Some((modification, vec![watched_path])),),
        );
    }

    #[cfg(unix)]
    #[test]
    fn output_event_watcher_rejects_symlink_targets_before_registration() {
        let directory = tempfile::tempdir().unwrap();
        let target_path = directory.path().join("target.output");
        let output_path = directory.path().join("current.output");
        fs::write(&target_path, "fixture output").unwrap();
        std::os::unix::fs::symlink(&target_path, &output_path).unwrap();

        let error = watch_output_events_with(output_path, |_, _| Ok(()))
            .err()
            .map(|error| error.to_string());

        assert_eq!(
            error,
            Some("background task output path is a symlink".to_owned()),
        );
    }

    #[cfg(unix)]
    #[test]
    fn background_task_output_path_is_canonicalized_through_symlinked_parent_directories() {
        let directory = tempfile::tempdir().unwrap();
        let actual_directory = directory.path().join("actual");
        let alias_directory = directory.path().join("alias");
        fs::create_dir(&actual_directory).unwrap();
        std::os::unix::fs::symlink(&actual_directory, &alias_directory).unwrap();
        let actual_path = actual_directory.join("output.log");
        let aliased_path = alias_directory.join("output.log");
        fs::write(&actual_path, "fixture output").unwrap();

        assert_eq!(
            canonical_output_path(&aliased_path).unwrap(),
            fs::canonicalize(&actual_path).unwrap(),
        );
    }

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
