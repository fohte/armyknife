use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use axum::Router;
use axum::body::Body;
use axum::extract::{Path as AxumPath, State};
use axum::http::header::{
    ACCEPT_RANGES, CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE,
    RANGE, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use base64::Engine as _;
use serde::Serialize;

use super::super::file::{self, storage_root};
use super::super::file_attachment_support::{
    FilePreviewType, MAX_AGENT_FILE_BYTES, file_preview_type, image_format_from_bytes,
    preview_type_from_mime, sanitize_basename,
};
use super::super::store::{self, AgentFile};
use super::super::types::{Engine, Session};

pub(super) const MAX_FILE_BYTES: u64 = MAX_AGENT_FILE_BYTES;

type SessionLoader = Arc<dyn Fn(&str) -> Result<Option<Session>> + Send + Sync>;
type FileLoader = Arc<dyn Fn(&str, &str) -> Result<Option<AgentFile>> + Send + Sync>;

#[derive(Clone)]
struct RoutesState {
    load_session: SessionLoader,
    load_file: FileLoader,
    storage_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ChatFile {
    pub(super) name: String,
    pub(super) source: String,
    pub(super) size: usize,
    pub(super) is_image: bool,
    pub(super) preview_type: FilePreviewType,
    pub(super) available: bool,
}

pub(super) fn routes() -> Router {
    let storage_root = storage_root().unwrap_or_else(|error| {
        tracing::warn!(%error, "failed to resolve agent file storage directory");
        PathBuf::new()
    });
    routes_with(
        store::load_session,
        |session_id, file_id| store::agent_file_from(&store::sessions_dir()?, session_id, file_id),
        storage_root,
    )
}

fn routes_with(
    load_session: impl Fn(&str) -> Result<Option<Session>> + Send + Sync + 'static,
    load_file: impl Fn(&str, &str) -> Result<Option<AgentFile>> + Send + Sync + 'static,
    storage_root: PathBuf,
) -> Router {
    let state = RoutesState {
        load_session: Arc::new(load_session),
        load_file: Arc::new(load_file),
        storage_root,
    };
    Router::new()
        .route(
            "/sessions/{session_id}/files/{file_id}",
            get(get_stored_file),
        )
        .route(
            "/sessions/{session_id}/transcript-files/{tool_use_id}/{index}",
            get(get_transcript_file),
        )
        .with_state(state)
}

pub(super) fn stored_chat_files(
    session_id: &str,
    records: &[AgentFile],
    storage_root: Option<&Path>,
) -> Vec<ChatFile> {
    records
        .iter()
        .map(|record| {
            let mut size = usize::try_from(record.size).unwrap_or_default();
            let preview_type = match record.mime_type.as_deref() {
                Some(mime_type) => preview_type_from_mime(&record.name, Some(mime_type)),
                None => storage_root
                    .and_then(|root| match file::read_stored_file_prefix(root, session_id, record) {
                        Ok(Some((bytes, stored_size))) => {
                            size = stored_size;
                            Some(file_preview_type(&record.name, &bytes))
                        }
                        Ok(None) => None,
                        Err(error) => {
                            tracing::warn!(%error, file_id = %record.id, "failed to classify stored agent file");
                            None
                        }
                    })
                    .unwrap_or(FilePreviewType::Download),
            };
            ChatFile {
                name: record.name.clone(),
                source: format!("/api/sessions/{session_id}/files/{}", record.id),
                size,
                is_image: preview_type == FilePreviewType::Image,
                preview_type,
                available: true,
            }
        })
        .collect()
}

pub(super) fn transcript_chat_file(
    session_id: &str,
    tool_use_id: &str,
    index: usize,
    path: &str,
) -> ChatFile {
    let path = Path::new(path);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(sanitize_basename)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "download".to_owned());
    let (available, size, preview_type) = match inspect_transcript_file(path) {
        Ok(Some((size, preview_type))) => (true, size, preview_type),
        Ok(None) => (false, 0, FilePreviewType::Download),
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "failed to check transcript attachment availability");
            (false, 0, FilePreviewType::Download)
        }
    };
    ChatFile {
        name,
        source: format!(
            "/api/sessions/{session_id}/transcript-files/{}/{index}",
            encode_tool_use_id(tool_use_id)
        ),
        size,
        is_image: preview_type == FilePreviewType::Image,
        preview_type,
        available,
    }
}

fn inspect_transcript_file(path: &Path) -> Result<Option<(usize, FilePreviewType)>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && metadata.len() <= MAX_FILE_BYTES => {}
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return Ok(None);
    }
    let size = metadata.len();
    let Some(bytes) = file::read_file_type_prefix(&mut file, size)? else {
        return Ok(None);
    };
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    Ok(Some((size as usize, file_preview_type(name, &bytes))))
}

async fn get_stored_file(
    headers: HeaderMap,
    State(state): State<RoutesState>,
    AxumPath((session_id, file_id)): AxumPath<(String, String)>,
) -> Response {
    let load_file = Arc::clone(&state.load_file);
    let storage_root = state.storage_root;
    let result = tokio::task::spawn_blocking(move || -> Result<Option<(String, Vec<u8>)>> {
        let Some(record) = load_file(&session_id, &file_id)? else {
            return Ok(None);
        };
        Ok(file::read_stored_file(&storage_root, &session_id, &record)?
            .map(|bytes| (record.name, bytes)))
    })
    .await;
    match result {
        Ok(Ok(Some((name, bytes)))) => file_response(&name, &bytes, headers.get(RANGE)),
        Ok(Ok(None)) => StatusCode::NOT_FOUND.into_response(),
        Ok(Err(error)) => {
            tracing::warn!(%error, "failed to read stored agent file");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
        Err(error) => {
            tracing::warn!(%error, "stored agent file task failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn get_transcript_file(
    headers: HeaderMap,
    State(state): State<RoutesState>,
    AxumPath((session_id, encoded_tool_use_id, index)): AxumPath<(String, String, usize)>,
) -> Response {
    let Ok(tool_use_bytes) =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(encoded_tool_use_id)
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(tool_use_id) = String::from_utf8(tool_use_bytes) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let session = match (state.load_session)(&session_id) {
        Ok(Some(session)) if session.engine == Engine::Claude => session,
        Ok(_) => return StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::warn!(%error, "failed to load session for transcript attachment");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let Some(transcript_path) = session.transcript_path.as_deref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let transcript_path = PathBuf::from(transcript_path);
    let file = match tokio::task::spawn_blocking(move || {
        read_transcript_file(&transcript_path, &tool_use_id, index)
    })
    .await
    {
        Ok(Ok(file)) => file,
        Ok(Err(error)) => {
            tracing::warn!(%error, "failed to read transcript attachment");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Err(error) => {
            tracing::warn!(%error, "transcript attachment task failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    match file {
        Some((path, bytes)) => {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(sanitize_basename)
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "download".to_owned());
            file_response(&name, &bytes, headers.get(RANGE))
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn read_transcript_file(
    transcript_path: &Path,
    tool_use_id: &str,
    index: usize,
) -> Result<Option<(PathBuf, Vec<u8>)>> {
    let Some(path) = transcript_file_path(transcript_path, tool_use_id, index)? else {
        return Ok(None);
    };
    Ok(read_file_for_delivery(&path)?.map(|bytes| (path, bytes)))
}

fn encode_tool_use_id(tool_use_id: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(tool_use_id)
}

fn transcript_file_path(
    transcript_path: &Path,
    expected_tool_use_id: &str,
    expected_index: usize,
) -> Result<Option<PathBuf>> {
    let file = match open_transcript(transcript_path)? {
        Some(file) => file,
        None => return Ok(None),
    };
    for line in file.lines() {
        let line = line?;
        let Ok(record) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let Some(content) = record
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for block in content {
            if block.get("type").and_then(serde_json::Value::as_str) != Some("tool_use")
                || block.get("name").and_then(serde_json::Value::as_str) != Some("SendUserFile")
                || block.get("id").and_then(serde_json::Value::as_str) != Some(expected_tool_use_id)
            {
                continue;
            }
            let Some(paths) = block
                .get("input")
                .and_then(|input| input.get("files"))
                .and_then(serde_json::Value::as_array)
            else {
                return Ok(None);
            };
            return Ok(paths
                .get(expected_index)
                .and_then(serde_json::Value::as_str)
                .map(PathBuf::from));
        }
    }
    Ok(None)
}

fn open_transcript(path: &Path) -> Result<Option<Box<dyn BufRead>>> {
    let (transcript, actual_path) = match File::open(path) {
        Ok(file) => (file, path.to_path_buf()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let compressed_path = compressed_path(path);
            match File::open(&compressed_path) {
                Ok(file) => (file, compressed_path),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error.into()),
            }
        }
        Err(error) => return Err(error.into()),
    };
    if actual_path
        .extension()
        .and_then(|extension| extension.to_str())
        == Some("zst")
    {
        return Ok(Some(Box::new(BufReader::new(
            zstd::stream::read::Decoder::new(transcript)?,
        ))));
    }
    Ok(Some(Box::new(BufReader::new(transcript))))
}

fn compressed_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".zst");
    PathBuf::from(name)
}

fn read_file_for_delivery(path: &Path) -> Result<Option<Vec<u8>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.len() > MAX_FILE_BYTES {
        return Ok(None);
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    options
        .open(path)?
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Ok(None);
    }
    Ok(Some(bytes))
}

fn file_response(name: &str, bytes: &[u8], range: Option<&HeaderValue>) -> Response {
    let preview_type = file_preview_type(name, bytes);
    let mime_type = match preview_type {
        FilePreviewType::Image => image_format_from_bytes(bytes)
            .map(|(mime_type, _)| mime_type)
            .unwrap_or("application/octet-stream"),
        FilePreviewType::Markdown => "text/markdown; charset=utf-8",
        FilePreviewType::Text => "text/plain; charset=utf-8",
        FilePreviewType::Video if name.to_ascii_lowercase().ends_with(".mov") => "video/quicktime",
        FilePreviewType::Video => "video/mp4",
        FilePreviewType::Audio if name.to_ascii_lowercase().ends_with(".m4a") => "audio/mp4",
        FilePreviewType::Audio => "audio/mpeg",
        FilePreviewType::Pdf => "application/pdf",
        FilePreviewType::Html | FilePreviewType::Download => "application/octet-stream",
    };
    let inline = !matches!(
        preview_type,
        FilePreviewType::Html | FilePreviewType::Download
    );
    let safe_name = sanitize_basename(name);
    let disposition = format!(
        "{}; filename=\"{safe_name}\"",
        if inline { "inline" } else { "attachment" }
    );
    let is_seekable = matches!(
        preview_type,
        FilePreviewType::Video | FilePreviewType::Audio
    );
    let selected_range = if is_seekable {
        range
            .and_then(|range| range.to_str().ok())
            .map(|range| parse_byte_range(range, bytes.len()))
    } else {
        None
    };

    let (status, body_bytes, content_range) = match selected_range {
        Some(Ok((start, end))) => (
            StatusCode::PARTIAL_CONTENT,
            bytes[start..=end].to_vec(),
            Some(format!("bytes {start}-{end}/{}", bytes.len())),
        ),
        Some(Err(ByteRangeError::Invalid)) => (
            StatusCode::RANGE_NOT_SATISFIABLE,
            Vec::new(),
            Some(format!("bytes */{}", bytes.len())),
        ),
        None => (StatusCode::OK, bytes.to_vec(), None),
    };

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(mime_type));
    if let Ok(value) = HeaderValue::from_str(&disposition) {
        headers.insert(CONTENT_DISPOSITION, value);
    }
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    if is_seekable {
        headers.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    }
    if let Some(value) = content_range.and_then(|value| HeaderValue::from_str(&value).ok()) {
        headers.insert(CONTENT_RANGE, value);
    }
    if let Ok(value) = HeaderValue::from_str(&body_bytes.len().to_string()) {
        headers.insert(CONTENT_LENGTH, value);
    }
    (status, headers, Body::from(body_bytes)).into_response()
}

#[derive(Debug)]
enum ByteRangeError {
    Invalid,
}

fn parse_byte_range(value: &str, length: usize) -> Result<(usize, usize), ByteRangeError> {
    let Some(range) = value.strip_prefix("bytes=") else {
        return Err(ByteRangeError::Invalid);
    };
    if range.contains(',') || length == 0 {
        return Err(ByteRangeError::Invalid);
    }
    let Some((start, end)) = range.split_once('-') else {
        return Err(ByteRangeError::Invalid);
    };
    if start.is_empty() {
        let suffix = end.parse::<usize>().map_err(|_| ByteRangeError::Invalid)?;
        if suffix == 0 {
            return Err(ByteRangeError::Invalid);
        }
        return Ok((length.saturating_sub(suffix), length - 1));
    }
    let start = start
        .parse::<usize>()
        .map_err(|_| ByteRangeError::Invalid)?;
    if start >= length {
        return Err(ByteRangeError::Invalid);
    }
    let end = if end.is_empty() {
        length - 1
    } else {
        end.parse::<usize>()
            .map_err(|_| ByteRangeError::Invalid)?
            .min(length - 1)
    };
    if end < start {
        return Err(ByteRangeError::Invalid);
    }
    Ok((start, end))
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use chrono::Utc;
    use rstest::rstest;
    use sha2::{Digest, Sha256};
    use tempfile::TempDir;
    use tower::ServiceExt;

    use super::{
        ChatFile, MAX_FILE_BYTES, encode_tool_use_id, file_response, routes_with,
        stored_chat_files, transcript_chat_file, transcript_file_path,
    };
    use crate::commands::agent::file_attachment_support::{
        FILE_TYPE_PREFIX_BYTES, FilePreviewType,
    };
    use crate::commands::agent::store::{self, AgentFile};
    use crate::commands::agent::types::Session;

    #[rstest]
    #[case::png("sample.PNG", &[137, 80, 78, 71, 13, 10, 26, 10])]
    #[case::jpeg("sample.JPEG", &[255, 216, 255])]
    #[case::gif("sample.GIF", b"GIF89a")]
    #[case::webp("sample.WEBP", b"RIFF0000WEBP")]
    #[case::extensionless("image", &[137, 80, 78, 71, 13, 10, 26, 10])]
    fn transcript_card_detects_images_from_content(#[case] name: &str, #[case] bytes: &[u8]) {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join(name);
        std::fs::write(&path, bytes).unwrap();

        let actual = transcript_chat_file(
            "fixture-session",
            "fixture-tool-use",
            0,
            path.to_str().unwrap(),
        );

        assert_eq!(
            actual,
            ChatFile {
                name: name.to_owned(),
                source: format!(
                    "/api/sessions/fixture-session/transcript-files/{}/0",
                    encode_tool_use_id("fixture-tool-use")
                ),
                size: bytes.len(),
                is_image: true,
                preview_type: FilePreviewType::Image,
                available: true,
            }
        );
    }

    #[test]
    fn transcript_card_does_not_trust_an_image_extension() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("not-an-image.png");
        std::fs::write(&path, b"not image content").unwrap();

        let actual = transcript_chat_file(
            "fixture-session",
            "fixture-tool-use",
            0,
            path.to_str().unwrap(),
        );

        assert_eq!(
            actual,
            ChatFile {
                name: "not-an-image.png".to_owned(),
                source: format!(
                    "/api/sessions/fixture-session/transcript-files/{}/0",
                    encode_tool_use_id("fixture-tool-use")
                ),
                size: b"not image content".len(),
                is_image: false,
                preview_type: FilePreviewType::Download,
                available: true,
            }
        );
    }

    #[test]
    fn transcript_card_marks_oversized_files_unavailable() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("large-report.pdf");
        std::fs::File::create(&path)
            .unwrap()
            .set_len(MAX_FILE_BYTES + 1)
            .unwrap();

        let actual = transcript_chat_file(
            "fixture-session",
            "fixture-tool-use",
            0,
            path.to_str().unwrap(),
        );

        assert_eq!(
            actual,
            ChatFile {
                name: "large-report.pdf".to_owned(),
                source: format!(
                    "/api/sessions/fixture-session/transcript-files/{}/0",
                    encode_tool_use_id("fixture-tool-use")
                ),
                size: 0,
                is_image: false,
                preview_type: FilePreviewType::Download,
                available: false,
            }
        );
    }

    #[test]
    fn transcript_card_uses_metadata_size_when_the_type_prefix_ends_mid_character() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("large-notes.md");
        let mut bytes = vec![b'a'; FILE_TYPE_PREFIX_BYTES - 1];
        bytes.push(0xe3);
        bytes.extend_from_slice(b"tail");
        std::fs::write(&path, &bytes).unwrap();

        let actual = transcript_chat_file(
            "fixture-session",
            "fixture-tool-use",
            0,
            path.to_str().unwrap(),
        );

        assert_eq!(
            actual,
            ChatFile {
                name: "large-notes.md".to_owned(),
                source: format!(
                    "/api/sessions/fixture-session/transcript-files/{}/0",
                    encode_tool_use_id("fixture-tool-use")
                ),
                size: bytes.len(),
                is_image: false,
                preview_type: FilePreviewType::Markdown,
                available: true,
            }
        );
    }

    #[test]
    fn stored_cards_use_mime_metadata_and_classify_legacy_file_bytes() {
        let storage = TempDir::new().unwrap();
        let session_id = "fixture-session";
        let storage_root = storage.path().join("attachments");
        let session_storage = storage_root.join(crate::shared::hex::encode(&Sha256::digest(
            session_id.as_bytes(),
        )));
        std::fs::create_dir_all(&session_storage).unwrap();
        let legacy_id = uuid::Uuid::new_v4().to_string();
        let legacy_bytes = b"# Legacy file";
        std::fs::write(session_storage.join(&legacy_id), legacy_bytes).unwrap();
        let created_at = Utc::now();
        let records = vec![
            AgentFile {
                id: "fixture-mime-id".to_owned(),
                position: 0,
                name: "sample-notes.md".to_owned(),
                caption: None,
                mime_type: Some("text/markdown".to_owned()),
                size: 11,
                created_at,
            },
            AgentFile {
                id: legacy_id.clone(),
                position: 1,
                name: "legacy-notes.md".to_owned(),
                caption: None,
                mime_type: None,
                size: legacy_bytes.len() as i64,
                created_at,
            },
        ];

        let actual = stored_chat_files(session_id, &records, Some(&storage_root));

        assert_eq!(
            actual,
            vec![
                ChatFile {
                    name: "sample-notes.md".to_owned(),
                    source: format!("/api/sessions/{session_id}/files/fixture-mime-id"),
                    size: 11,
                    is_image: false,
                    preview_type: FilePreviewType::Markdown,
                    available: true,
                },
                ChatFile {
                    name: "legacy-notes.md".to_owned(),
                    source: format!("/api/sessions/{session_id}/files/{legacy_id}"),
                    size: legacy_bytes.len(),
                    is_image: false,
                    preview_type: FilePreviewType::Markdown,
                    available: true,
                },
            ]
        );
    }

    #[tokio::test]
    async fn serves_video_byte_ranges_for_seeking() {
        let range = axum::http::HeaderValue::from_static("bytes=4-7");
        let response = file_response("clip.mp4", b"0000ftypabcd", Some(&range));
        let headers = response.headers();
        let actual = (
            response.status(),
            headers.get(axum::http::header::CONTENT_TYPE).cloned(),
            headers
                .get(axum::http::header::CONTENT_DISPOSITION)
                .cloned(),
            headers.get(axum::http::header::ACCEPT_RANGES).cloned(),
            headers.get(axum::http::header::CONTENT_RANGE).cloned(),
            headers.get(axum::http::header::CONTENT_LENGTH).cloned(),
            headers
                .get(axum::http::header::X_CONTENT_TYPE_OPTIONS)
                .cloned(),
            to_bytes(response.into_body(), MAX_FILE_BYTES as usize)
                .await
                .unwrap()
                .to_vec(),
        );

        assert_eq!(
            actual,
            (
                StatusCode::PARTIAL_CONTENT,
                Some("video/mp4".parse().unwrap()),
                Some("inline; filename=\"clip.mp4\"".parse().unwrap()),
                Some("bytes".parse().unwrap()),
                Some("bytes 4-7/12".parse().unwrap()),
                Some("4".parse().unwrap()),
                Some("nosniff".parse().unwrap()),
                b"ftyp".to_vec(),
            )
        );
    }

    #[tokio::test]
    async fn serves_html_as_a_download_with_a_non_html_content_type() {
        let response = file_response(
            "document.html",
            b"<!doctype html><script>window.example = true</script>",
            None,
        );
        let headers = response.headers();
        let actual = (
            response.status(),
            headers.get(axum::http::header::CONTENT_TYPE).cloned(),
            headers
                .get(axum::http::header::CONTENT_DISPOSITION)
                .cloned(),
            headers
                .get(axum::http::header::X_CONTENT_TYPE_OPTIONS)
                .cloned(),
        );

        assert_eq!(
            actual,
            (
                StatusCode::OK,
                Some("application/octet-stream".parse().unwrap()),
                Some("attachment; filename=\"document.html\"".parse().unwrap()),
                Some("nosniff".parse().unwrap()),
            )
        );
    }

    #[tokio::test]
    async fn serves_a_registered_file_and_rejects_an_unregistered_id() {
        let storage = TempDir::new().unwrap();
        let sessions = TempDir::new().unwrap();
        let session_id = "fixture-session";
        store::session_store_revision_from(sessions.path()).unwrap();
        rusqlite::Connection::open(sessions.path().join("sessions.db"))
            .unwrap()
            .execute(
                "INSERT INTO sessions (session_id, data, rev) VALUES (?1, '{}', 0)",
                [session_id],
            )
            .unwrap();
        let storage_root = storage.path().join("attachments");
        let file = AgentFile {
            id: uuid::Uuid::new_v4().to_string(),
            position: 0,
            name: "sample-report.pdf".to_owned(),
            caption: Some("A sample report".to_owned()),
            mime_type: None,
            size: b"fixture report".len() as i64,
            created_at: Utc::now(),
        };
        let session_storage = storage_root.join(crate::shared::hex::encode(&Sha256::digest(
            session_id.as_bytes(),
        )));
        std::fs::create_dir_all(&session_storage).unwrap();
        std::fs::write(session_storage.join(&file.id), b"fixture report").unwrap();
        store::insert_agent_files_from(sessions.path(), session_id, std::slice::from_ref(&file))
            .unwrap();
        let registered_file_id = file.id.clone();
        let request_file_id = file.id.clone();
        let app = routes_with(
            |_| Ok(None),
            move |requested_session, requested_file_id| {
                Ok(
                    (requested_session == session_id && requested_file_id == registered_file_id)
                        .then(|| file.clone()),
                )
            },
            storage_root,
        );

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/sessions/{session_id}/files/{request_file_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let content_type = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let content_disposition = response
            .headers()
            .get(axum::http::header::CONTENT_DISPOSITION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = to_bytes(response.into_body(), MAX_FILE_BYTES as usize)
            .await
            .unwrap();
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/sessions/{session_id}/files/{}",
                        "00000000-0000-4000-8000-000000000002"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            (
                status,
                content_type,
                content_disposition,
                String::from_utf8(body.to_vec()).unwrap(),
                response.status()
            ),
            (
                StatusCode::OK,
                Some("application/octet-stream".to_owned()),
                Some("attachment; filename=\"sample-report.pdf\"".to_owned()),
                "fixture report".to_owned(),
                StatusCode::NOT_FOUND
            )
        );
    }

    #[tokio::test]
    async fn serves_only_files_listed_by_the_session_send_user_file_tool_use() {
        let directory = TempDir::new().unwrap();
        let transcript_path = directory.path().join("conversation.jsonl");
        let image_path = directory.path().join("sample.png");
        std::fs::write(&image_path, [137, 80, 78, 71, 13, 10, 26, 10]).unwrap();
        std::fs::write(
            &transcript_path,
            serde_json::json!({
                "type": "assistant",
                "message": {
                    "content": [{
                        "type": "tool_use",
                        "id": "fixture-tool-use",
                        "name": "SendUserFile",
                        "input": { "files": [image_path.to_string_lossy()] }
                    }]
                }
            })
            .to_string(),
        )
        .unwrap();
        let session: Session = serde_json::from_value(serde_json::json!({
            "session_id": "fixture-session",
            "cwd": "/fixture/project",
            "status": "ended",
            "created_at": "2030-01-01T00:00:00Z",
            "updated_at": "2030-01-01T00:00:00Z",
            "transcript_path": transcript_path
        }))
        .unwrap();
        let app = routes_with(
            move |requested_session| {
                Ok((requested_session == "fixture-session").then(|| session.clone()))
            },
            |_, _| Ok(None),
            directory.path().to_path_buf(),
        );
        let source = format!(
            "/sessions/fixture-session/transcript-files/{}/0",
            encode_tool_use_id("fixture-tool-use")
        );
        let valid_response = app
            .clone()
            .oneshot(Request::builder().uri(source).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let valid_status = valid_response.status();
        let valid_content_type = valid_response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let valid_body = to_bytes(valid_response.into_body(), MAX_FILE_BYTES as usize)
            .await
            .unwrap();
        let unlisted_response = app
            .oneshot(
                Request::builder()
                    .uri("/sessions/fixture-session/transcript-files/b3RoZXItdG9vbC11c2U/0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            (
                valid_status,
                valid_content_type,
                valid_body.to_vec(),
                unlisted_response.status()
            ),
            (
                StatusCode::OK,
                Some("image/png".to_owned()),
                vec![137, 80, 78, 71, 13, 10, 26, 10],
                StatusCode::NOT_FOUND
            )
        );
    }

    #[rstest]
    #[case::listed_file_is_resolved("fixture-tool-use", 0, Some("/fixture/report.pdf"), false)]
    #[case::compressed_transcript_is_resolved(
        "fixture-tool-use",
        0,
        Some("/fixture/report.pdf"),
        true
    )]
    #[case::different_tool_use_is_rejected("other-tool-use", 0, None, false)]
    #[case::unlisted_index_is_rejected("fixture-tool-use", 1, None, false)]
    fn only_send_user_file_paths_are_resolved(
        #[case] tool_use_id: &str,
        #[case] index: usize,
        #[case] expected_path: Option<&str>,
        #[case] compressed: bool,
    ) {
        let transcript = TempDir::new().unwrap();
        let transcript_path = transcript.path().join("conversation.jsonl");
        let record = serde_json::json!({
            "type": "assistant",
            "message": {
                "content": [{
                    "type": "tool_use",
                    "id": "fixture-tool-use",
                    "name": "SendUserFile",
                    "input": { "files": ["/fixture/report.pdf"] }
                }]
            }
        })
        .to_string();
        if compressed {
            std::fs::write(
                super::compressed_path(&transcript_path),
                zstd::stream::encode_all(record.as_bytes(), 0).unwrap(),
            )
            .unwrap();
        } else {
            std::fs::write(&transcript_path, record).unwrap();
        }

        let actual = transcript_file_path(&transcript_path, tool_use_id, index)
            .unwrap()
            .map(|path| path.to_string_lossy().into_owned());

        assert_eq!(actual.as_deref(), expected_path);
    }
}
