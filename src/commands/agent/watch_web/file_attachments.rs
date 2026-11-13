use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use axum::Router;
use axum::body::Body;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::http::header::{
    CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS,
};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use base64::Engine as _;
use serde::Serialize;

use super::super::file::{self, storage_root};
use super::super::file_attachment_support::{
    MAX_AGENT_FILE_BYTES, image_format_from_bytes, is_image_mime_type, sanitize_basename,
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
    pub(super) is_image: bool,
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

pub(super) fn stored_chat_files(session_id: &str, records: &[AgentFile]) -> Vec<ChatFile> {
    records
        .iter()
        .map(|record| ChatFile {
            name: record.name.clone(),
            source: format!("/api/sessions/{session_id}/files/{}", record.id),
            is_image: record.mime_type.as_deref().is_some_and(is_image_mime_type),
            available: true,
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
    let (available, is_image) = match inspect_transcript_file(path) {
        Ok(Some(is_image)) => (true, is_image),
        Ok(None) => (false, false),
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "failed to check transcript attachment availability");
            (false, false)
        }
    };
    ChatFile {
        name,
        source: format!(
            "/api/sessions/{session_id}/transcript-files/{}/{index}",
            encode_tool_use_id(tool_use_id)
        ),
        is_image,
        available,
    }
}

fn inspect_transcript_file(path: &Path) -> Result<Option<bool>> {
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
    let mut prefix = Vec::with_capacity(12);
    file.by_ref().take(12).read_to_end(&mut prefix)?;
    Ok(Some(image_format_from_bytes(&prefix).is_some()))
}

async fn get_stored_file(
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
        Ok(Ok(Some((name, bytes)))) => file_response(&name, &bytes),
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
            file_response(&name, &bytes)
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

fn file_response(name: &str, bytes: &[u8]) -> Response {
    let mime_type = image_format_from_bytes(bytes).map(|(mime_type, _)| mime_type);
    let (mime_type, disposition) = match mime_type {
        Some(mime_type) => (mime_type, format!("inline; filename=\"{name}\"")),
        None => (
            "application/octet-stream",
            format!("attachment; filename=\"{name}\""),
        ),
    };
    (
        StatusCode::OK,
        [
            (CONTENT_TYPE, mime_type.to_owned()),
            (CONTENT_DISPOSITION, disposition),
            (CACHE_CONTROL, "private, no-store".to_owned()),
            (X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
        ],
        Body::from(bytes.to_vec()),
    )
        .into_response()
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
        ChatFile, MAX_FILE_BYTES, encode_tool_use_id, routes_with, transcript_chat_file,
        transcript_file_path,
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
                is_image: true,
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
                is_image: false,
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
                is_image: false,
                available: false,
            }
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
