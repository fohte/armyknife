use axum::http::StatusCode;

use crate::shared::human_in_the_loop::{
    ReviewDocumentOperationLock, finish_ack_fifo_path, finish_request_fifo_path, open_fifo_reader,
    signal_review_finish_and_wait,
};

use super::{
    ReviewDocumentEditRequest, ReviewQueueLoader, SessionLoader, active_review_document,
    review_document_revision, select_review_document,
};

pub(super) async fn save_document_for_session(
    load_session: SessionLoader,
    load_review_queue: ReviewQueueLoader,
    session_id: String,
    request: ReviewDocumentEditRequest,
    finishing: bool,
) -> Result<String, StatusCode> {
    tokio::task::spawn_blocking(move || {
        let session = load_session(&session_id)
            .map_err(|error| {
                tracing::warn!(%error, "failed to load session for review document edit");
                StatusCode::INTERNAL_SERVER_ERROR
            })?
            .ok_or(StatusCode::NOT_FOUND)?;
        let queue = load_review_queue(&session);
        let marker = active_review_document(&session, queue).ok_or(StatusCode::NOT_FOUND)?;
        if marker.id != request.id {
            return Err(StatusCode::CONFLICT);
        }
        let review_lock = ReviewDocumentOperationLock::acquire(&marker.path).map_err(|error| {
            tracing::warn!(%error, "failed to lock active review document for editing");
            if error.kind() == std::io::ErrorKind::NotFound {
                StatusCode::CONFLICT
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        })?;
        let current_session = load_session(&session_id)
            .map_err(|error| {
                tracing::warn!(%error, "failed to recheck session for review document edit");
                StatusCode::INTERNAL_SERVER_ERROR
            })?
            .ok_or(StatusCode::NOT_FOUND)?;
        let current_marker =
            select_review_document(&current_session, load_review_queue(&current_session))
                .ok_or(StatusCode::NOT_FOUND)?;
        if current_marker.id != request.id
            || current_marker.path != marker.path
            || !review_lock.is_active(&marker.path)
        {
            return Err(StatusCode::CONFLICT);
        }
        let current_content = std::fs::read(&marker.path).map_err(|error| {
            tracing::warn!(%error, "failed to read active review document before saving");
            if error.kind() == std::io::ErrorKind::NotFound {
                StatusCode::CONFLICT
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        })?;
        if request.revision != review_document_revision(&current_content) {
            return Err(StatusCode::CONFLICT);
        }
        let fifo_path = finish_request_fifo_path(&marker.path);
        if !is_fifo(&fifo_path) {
            return Err(StatusCode::CONFLICT);
        }
        let mut ack_reader = if finishing {
            Some(
                open_fifo_reader(&finish_ack_fifo_path(&marker.path)).map_err(|error| {
                    tracing::warn!(%error, "failed to open review finish acknowledgement FIFO");
                    StatusCode::INTERNAL_SERVER_ERROR
                })?,
            )
        } else {
            None
        };
        let content = render_review_document(&marker.path, request.frontmatter, &request.body)?;
        let revision = review_document_revision(content.as_bytes());
        overwrite_review_document(&marker.path, content.as_bytes(), &request.revision)?;
        let acknowledgement = match ack_reader.as_mut() {
            Some(ack_reader) => Some(
                signal_review_finish_and_wait(review_lock, &fifo_path, ack_reader).map_err(
                    |error| {
                        tracing::warn!(%error, "review process did not acknowledge finish request");
                        if error.kind() == std::io::ErrorKind::TimedOut {
                            StatusCode::GATEWAY_TIMEOUT
                        } else {
                            StatusCode::INTERNAL_SERVER_ERROR
                        }
                    },
                )?,
            ),
            None => {
                drop(review_lock);
                None
            }
        };
        if let Some(acknowledgement) = acknowledgement
            && acknowledgement != b'0'
        {
            tracing::warn!("review completion handler rejected the finish request");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
        Ok(revision)
    })
    .await
    .map_err(|error| {
        tracing::error!(%error, "review document edit task failed");
        StatusCode::INTERNAL_SERVER_ERROR
    })?
}

fn overwrite_review_document(
    path: &std::path::Path,
    content: &[u8],
    expected_revision: &str,
) -> Result<(), StatusCode> {
    use std::fs;
    use std::io::Write;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    let metadata = fs::symlink_metadata(path).map_err(|error| {
        tracing::warn!(%error, "failed to inspect active review document before saving");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    if !metadata.file_type().is_file() {
        return Err(StatusCode::CONFLICT);
    }

    let mut staging_name = path.as_os_str().to_os_string();
    staging_name.push(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let staging_path = std::path::PathBuf::from(staging_name);
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .mode(metadata.mode() & 0o7777)
            .custom_flags(libc::O_NOFOLLOW);
        let mut file = options.open(&staging_path).map_err(|error| {
            tracing::warn!(%error, "failed to create review document staging file");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
        file.set_permissions(metadata.permissions())
            .map_err(|error| {
                tracing::warn!(%error, "failed to preserve review document permissions");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
        file.write_all(content).map_err(|error| {
            tracing::warn!(%error, "failed to write review document staging file");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
        file.sync_all().map_err(|error| {
            tracing::warn!(%error, "failed to sync review document staging file");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

        let current_content = fs::read(path).map_err(|error| {
            tracing::warn!(%error, "failed to recheck active review document before saving");
            if error.kind() == std::io::ErrorKind::NotFound {
                StatusCode::CONFLICT
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        })?;
        if review_document_revision(&current_content) != expected_revision {
            return Err(StatusCode::CONFLICT);
        }

        let current_metadata = fs::symlink_metadata(path).map_err(|error| {
            tracing::warn!(%error, "failed to recheck active review document before saving");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
        if !current_metadata.file_type().is_file()
            || current_metadata.dev() != metadata.dev()
            || current_metadata.ino() != metadata.ino()
        {
            return Err(StatusCode::CONFLICT);
        }

        fs::rename(&staging_path, path).map_err(|error| {
            tracing::warn!(%error, "failed to replace active review document");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
        let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| {
                tracing::warn!(%error, "failed to sync active review document directory");
                StatusCode::INTERNAL_SERVER_ERROR
            })
    })();

    if result.is_err() {
        let _ = fs::remove_file(staging_path);
    }
    result
}

fn render_review_document(
    path: &std::path::Path,
    frontmatter: serde_json::Value,
    body: &str,
) -> Result<String, StatusCode> {
    let current = std::fs::read_to_string(path).map_err(|error| {
        tracing::warn!(%error, "failed to read active review document before saving");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let mut content = String::new();
    if crate::shared::yaml_frontmatter::split_frontmatter(&current).is_some()
        || !frontmatter.is_null()
    {
        let yaml = serde_yaml::to_string(&frontmatter).map_err(|error| {
            tracing::warn!(%error, "failed to serialize review document frontmatter");
            StatusCode::BAD_REQUEST
        })?;
        content.push_str("---\n");
        content.push_str(yaml.trim_start_matches("---\n").trim_end());
        content.push('\n');
        content.push_str("---");
        content.push('\n');
    }
    content.push_str(body);
    Ok(content)
}

#[cfg(unix)]
pub(super) fn is_fifo(path: &std::path::Path) -> bool {
    use std::os::unix::fs::FileTypeExt;

    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_fifo())
}

#[cfg(not(unix))]
pub(super) fn is_fifo(_path: &std::path::Path) -> bool {
    false
}
