use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{DefaultBodyLimit, Path as RoutePath, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};

use crate::commands::agent::review_queue::{ReviewDocumentQueue, review_document_queue};
use crate::commands::agent::store;
use crate::commands::agent::types::Session;
use crate::shared::human_in_the_loop::{
    FINISH_ACK_TIMEOUT, LockGuard, ReviewDocumentMarker, ReviewDocumentOperationLock,
    finish_ack_fifo_path, finish_request_fifo_path, open_fifo_reader, review_document_marker,
    wait_for_finish_ack,
};

type SessionLoader = Arc<dyn Fn(&str) -> anyhow::Result<Option<Session>> + Send + Sync>;
type ReviewQueueLoader = Arc<dyn Fn(&Session) -> ReviewDocumentQueue + Send + Sync>;
type ReviewDocumentEditLock = Arc<tokio::sync::Mutex<()>>;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewDocumentResponse {
    id: String,
    frontmatter: serde_json::Value,
    body: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewDocumentEditRequest {
    id: String,
    frontmatter: serde_json::Value,
    body: String,
}

#[derive(Serialize)]
struct ReviewDocumentSaved {
    success: bool,
}

pub(super) fn router() -> Router {
    router_with_dependencies(store::load_session, review_document_queue)
}

#[cfg(test)]
fn router_with_session_loader(
    load_session: impl Fn(&str) -> anyhow::Result<Option<Session>> + Send + Sync + 'static,
) -> Router {
    router_with_loaders(load_session, |_| ReviewDocumentQueue::Unmanaged)
}

#[cfg(test)]
fn router_with_loaders(
    load_session: impl Fn(&str) -> anyhow::Result<Option<Session>> + Send + Sync + 'static,
    load_review_queue: impl Fn(&Session) -> ReviewDocumentQueue + Send + Sync + 'static,
) -> Router {
    router_with_dependencies(load_session, load_review_queue)
}

fn router_with_dependencies(
    load_session: impl Fn(&str) -> anyhow::Result<Option<Session>> + Send + Sync + 'static,
    load_review_queue: impl Fn(&Session) -> ReviewDocumentQueue + Send + Sync + 'static,
) -> Router {
    Router::new()
        .route("/sessions/{id}/review-document", get(get_review_document))
        .route(
            "/sessions/{id}/review-document/save",
            post(save_review_document),
        )
        .route(
            "/sessions/{id}/review-document/finish",
            post(finish_review_document),
        )
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
        .with_state((
            Arc::new(load_session) as SessionLoader,
            Arc::new(load_review_queue) as ReviewQueueLoader,
            Arc::new(tokio::sync::Mutex::new(())) as ReviewDocumentEditLock,
        ))
}

async fn get_review_document(
    State((load_session, load_review_queue, _)): State<(
        SessionLoader,
        ReviewQueueLoader,
        ReviewDocumentEditLock,
    )>,
    RoutePath(session_id): RoutePath<String>,
) -> Result<Json<ReviewDocumentResponse>, StatusCode> {
    let response = tokio::task::spawn_blocking(move || {
        let session = match load_session(&session_id) {
            Ok(Some(session)) => session,
            Ok(None) => return Ok(None),
            Err(error) => {
                tracing::warn!(%error, "failed to load session for review document");
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
        };
        if !has_review_document_marker(&session) {
            return Ok(None);
        }
        let queue = load_review_queue(&session);
        let Some(marker) = active_review_document(&session, queue) else {
            return Ok(None);
        };
        match load_review_document(&marker) {
            Ok(response) => Ok(Some(response)),
            Err(error) => {
                tracing::warn!(%error, "failed to read active review document");
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    })
    .await
    .map_err(|error| {
        tracing::error!(%error, "review document task failed");
        StatusCode::INTERNAL_SERVER_ERROR
    })??;

    response.map(Json).ok_or(StatusCode::NOT_FOUND)
}

async fn save_review_document(
    State((load_session, load_review_queue, edit_lock)): State<(
        SessionLoader,
        ReviewQueueLoader,
        ReviewDocumentEditLock,
    )>,
    RoutePath(session_id): RoutePath<String>,
    axum::Json(request): axum::Json<ReviewDocumentEditRequest>,
) -> Result<axum::Json<ReviewDocumentSaved>, StatusCode> {
    let _edit_guard = edit_lock.lock().await;
    save_document_for_session(load_session, load_review_queue, session_id, request, false).await?;
    Ok(axum::Json(ReviewDocumentSaved { success: true }))
}

async fn finish_review_document(
    State((load_session, load_review_queue, edit_lock)): State<(
        SessionLoader,
        ReviewQueueLoader,
        ReviewDocumentEditLock,
    )>,
    RoutePath(session_id): RoutePath<String>,
    axum::Json(request): axum::Json<ReviewDocumentEditRequest>,
) -> Result<axum::Json<ReviewDocumentSaved>, StatusCode> {
    let _edit_guard = edit_lock.lock().await;
    save_document_for_session(load_session, load_review_queue, session_id, request, true).await?;
    Ok(axum::Json(ReviewDocumentSaved { success: true }))
}

async fn save_document_for_session(
    load_session: SessionLoader,
    load_review_queue: ReviewQueueLoader,
    session_id: String,
    request: ReviewDocumentEditRequest,
    finishing: bool,
) -> Result<(), StatusCode> {
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
        overwrite_review_document(&marker.path, content.as_bytes())?;
        if finishing {
            crate::shared::human_in_the_loop::try_signal_fifo(&fifo_path).map_err(|error| {
                tracing::warn!(%error, "failed to signal review finish request");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
        }
        drop(review_lock);
        if let Some(ack_reader) = ack_reader.as_mut() {
            let acknowledgement =
                wait_for_finish_ack(ack_reader, FINISH_ACK_TIMEOUT).map_err(|error| {
                    tracing::warn!(%error, "review process did not acknowledge finish request");
                    if error.kind() == std::io::ErrorKind::TimedOut {
                        StatusCode::CONFLICT
                    } else {
                        StatusCode::INTERNAL_SERVER_ERROR
                    }
                })?;
            if acknowledgement != b'0' {
                tracing::warn!("review completion handler rejected the finish request");
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
        }
        Ok(())
    })
    .await
    .map_err(|error| {
        tracing::error!(%error, "review document edit task failed");
        StatusCode::INTERNAL_SERVER_ERROR
    })?
}

fn overwrite_review_document(path: &std::path::Path, content: &[u8]) -> Result<(), StatusCode> {
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
fn is_fifo(path: &std::path::Path) -> bool {
    use std::os::unix::fs::FileTypeExt;

    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_fifo())
}

#[cfg(not(unix))]
fn is_fifo(_path: &std::path::Path) -> bool {
    false
}

pub(super) fn active_review_document_id(
    session: &Session,
    queue: ReviewDocumentQueue,
) -> Option<String> {
    if !has_review_document_marker(session) {
        return None;
    }
    active_review_document(session, queue).map(|marker| marker.id)
}

pub(super) fn has_review_document_marker(session: &Session) -> bool {
    session
        .pending_human_review_ids
        .iter()
        .any(|review_id| review_document_marker(review_id).is_some())
}

pub(super) fn active_review_document_ids(
    sessions: &[Session],
    load_review_queue: impl Fn(&Session) -> ReviewDocumentQueue,
) -> HashMap<String, Option<String>> {
    sessions
        .iter()
        .filter_map(|session| {
            if !has_review_document_marker(session) {
                return None;
            }
            let review_document_id = active_review_document_id(session, load_review_queue(session));
            Some((session.session_id.clone(), review_document_id))
        })
        .collect()
}

fn active_review_document(
    session: &Session,
    queue: ReviewDocumentQueue,
) -> Option<ReviewDocumentMarker> {
    let marker = select_review_document(session, queue)?;
    LockGuard::is_locked(&marker.path).then_some(marker)
}

fn select_review_document(
    session: &Session,
    queue: ReviewDocumentQueue,
) -> Option<ReviewDocumentMarker> {
    let mut markers = session
        .pending_human_review_ids
        .iter()
        .filter_map(|review_id| review_document_marker(review_id))
        .collect::<Vec<_>>();

    match queue {
        ReviewDocumentQueue::Managed(Some(shown_key)) => markers
            .into_iter()
            .find(|marker| marker.queue_key.as_deref() == Some(shown_key.as_str())),
        ReviewDocumentQueue::Managed(None) | ReviewDocumentQueue::Unavailable => None,
        ReviewDocumentQueue::Unmanaged => {
            markers.retain(|marker| marker.queue_key.is_none());
            markers.sort_by(|left, right| {
                left.started_at
                    .cmp(&right.started_at)
                    .then_with(|| left.id.cmp(&right.id))
            });
            markers.into_iter().next()
        }
    }
}

fn load_review_document(marker: &ReviewDocumentMarker) -> anyhow::Result<ReviewDocumentResponse> {
    let content = std::fs::read_to_string(&marker.path)?;
    let (frontmatter, body) = match crate::shared::yaml_frontmatter::split_frontmatter(&content) {
        Some((_, yaml, body_offset)) => (
            serde_json::to_value(serde_yaml::from_str::<serde_yaml::Value>(yaml)?)?,
            content[body_offset..].to_owned(),
        ),
        None => (serde_json::Value::Null, content),
    };

    Ok(ReviewDocumentResponse {
        id: marker.id.clone(),
        frontmatter,
        body,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use chrono::Utc;
    use indoc::indoc;
    use rstest::{fixture, rstest};
    use tempfile::TempDir;
    use tower::ServiceExt;

    use crate::commands::agent::review_queue::ReviewDocumentQueue;
    use crate::commands::agent::types::{Engine, Session, SessionStatus};
    use crate::shared::human_in_the_loop::LockGuard;

    use super::{
        ReviewDocumentOperationLock, finish_ack_fifo_path, finish_request_fifo_path,
        router_with_loaders, router_with_session_loader,
    };

    struct ActiveReviewDocument {
        temp_dir: TempDir,
        document_path: PathBuf,
        session: Session,
        _review_lock: LockGuard,
    }

    fn review_marker(
        id: &str,
        path: &std::path::Path,
        started_at: u128,
        queue_key: Option<&str>,
    ) -> String {
        format!(
            "{id}\0v2\0{started_at}\0{}\0{}",
            path.display(),
            queue_key.unwrap_or_default()
        )
    }

    fn session_with_review(path: &std::path::Path) -> Session {
        Session {
            session_id: "fixture-session".to_string(),
            archived_at: None,
            pr_context: None,
            crit_urls: Vec::new(),
            web_uis: Vec::new(),
            pending_human_review_ids: BTreeSet::from([review_marker(
                "fixture-review",
                path,
                1,
                None,
            )]),
            cwd: PathBuf::from("/fixture/workspace"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Stopped,
            agent_status: None,
            agent_status_clear_pending: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
            current_tool_name: None,
            label: None,
            work_type: None,
            vibe_enabled: false,
            launched_with_vibe: false,
            work_type_pinned: false,
            child_launch: Default::default(),
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: BTreeSet::new(),
            pending_agent_task_ids: BTreeSet::new(),
            pending_permission_agent_ids: BTreeSet::new(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            engine: Engine::Claude,
            model: None,
            effort: None,
        }
    }

    #[cfg(unix)]
    fn create_fixture_fifo(path: &std::path::Path) {
        let fifo_name = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
            .expect("fixture FIFO path should not contain NUL");
        // SAFETY: The path is inside this test's temporary directory.
        let result = unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o600) };
        assert_eq!(result, 0, "fixture FIFO should be created");
    }

    #[fixture]
    fn active_review_document() -> ActiveReviewDocument {
        let temp_dir = TempDir::new().expect("fixture directory should be created");
        let document_path = temp_dir.path().join("fixture-document.md");
        std::fs::write(&document_path, "Original body.")
            .expect("fixture document should be written");
        #[cfg(unix)]
        {
            create_fixture_fifo(&finish_request_fifo_path(&document_path));
            create_fixture_fifo(&finish_ack_fifo_path(&document_path));
        }
        let session = session_with_review(&document_path);
        let review_lock =
            LockGuard::acquire(&document_path).expect("fixture review lock should be acquired");

        ActiveReviewDocument {
            temp_dir,
            document_path,
            session,
            _review_lock: review_lock,
        }
    }

    #[tokio::test]
    async fn returns_the_active_review_document_frontmatter_and_markdown_body() {
        let temp_dir = TempDir::new().expect("fixture directory should be created");
        let document_path = temp_dir.path().join("fixture-document.md");
        std::fs::write(
            &document_path,
            indoc! {"
                ---
                summary: Fixture summary
                enabled: true
                labels:
                  - first
                  - second
                settings:
                  retries: 2
                  enabled: false
                ---
                ## Overview

                Fixture body.
            "},
        )
        .expect("fixture document should be written");
        let session = session_with_review(&document_path);
        let _review_lock =
            LockGuard::acquire(&document_path).expect("fixture review lock should be acquired");
        let app = router_with_session_loader(move |_| Ok(Some(session.clone())));
        let response = app
            .oneshot(
                Request::get("/sessions/fixture-session/review-document?path=/client/supplied.md")
                    .body(Body::empty())
                    .expect("request should be valid"),
            )
            .await
            .expect("request should complete");
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body should be read");

        assert_eq!(
            (
                status,
                serde_json::from_slice::<serde_json::Value>(&body)
                    .expect("response should be JSON"),
            ),
            (
                StatusCode::OK,
                serde_json::json!({
                    "id": "fixture-review",
                    "frontmatter": {
                        "summary": "Fixture summary",
                        "enabled": true,
                        "labels": ["first", "second"],
                        "settings": {
                            "retries": 2,
                            "enabled": false,
                        },
                    },
                    "body": indoc! {"
                        ## Overview

                        Fixture body.
                    "},
                }),
            ),
        );
    }

    #[tokio::test]
    async fn returns_the_document_shown_by_the_review_queue() {
        let temp_dir = TempDir::new().expect("fixture directory should be created");
        let visible_path = temp_dir.path().join("visible-document.md");
        let hidden_path = temp_dir.path().join("hidden-document.md");
        std::fs::write(&visible_path, "Visible body.").expect("visible document should be written");
        std::fs::write(&hidden_path, "Hidden body.").expect("hidden document should be written");
        let _visible_lock =
            LockGuard::acquire(&visible_path).expect("visible review lock should be acquired");
        let _hidden_lock =
            LockGuard::acquire(&hidden_path).expect("hidden review lock should be acquired");
        let mut session = session_with_review(&visible_path);
        session.pending_human_review_ids = BTreeSet::from([
            review_marker("visible-review", &visible_path, 1, Some("visible-session")),
            review_marker("hidden-review", &hidden_path, 2, Some("hidden-session")),
        ]);
        let app = router_with_loaders(
            move |_| Ok(Some(session.clone())),
            |_| ReviewDocumentQueue::Managed(Some("visible-session".to_string())),
        );
        let response = app
            .oneshot(
                Request::get("/sessions/fixture-session/review-document")
                    .body(Body::empty())
                    .expect("request should be valid"),
            )
            .await
            .expect("request should complete");
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body should be read");

        assert_eq!(
            (
                status,
                serde_json::from_slice::<serde_json::Value>(&body)
                    .expect("response should be JSON"),
            ),
            (
                StatusCode::OK,
                serde_json::json!({
                    "id": "visible-review",
                    "frontmatter": null,
                    "body": "Visible body.",
                }),
            ),
        );
    }

    #[tokio::test]
    async fn returns_the_oldest_review_document_for_unmanaged_sessions() {
        let temp_dir = TempDir::new().expect("fixture directory should be created");
        let older_path = temp_dir.path().join("older-document.md");
        let newer_path = temp_dir.path().join("newer-document.md");
        std::fs::write(&older_path, "Earlier body.").expect("older document should be written");
        std::fs::write(&newer_path, "Later body.").expect("newer document should be written");
        let _older_lock =
            LockGuard::acquire(&older_path).expect("older review lock should be acquired");
        let _newer_lock =
            LockGuard::acquire(&newer_path).expect("newer review lock should be acquired");
        let mut session = session_with_review(&newer_path);
        session.pending_human_review_ids = BTreeSet::from([
            review_marker("review-a-newer", &newer_path, 20, None),
            review_marker("review-z-older", &older_path, 10, None),
        ]);
        let app = router_with_session_loader(move |_| Ok(Some(session.clone())));
        let response = app
            .oneshot(
                Request::get("/sessions/fixture-session/review-document")
                    .body(Body::empty())
                    .expect("request should be valid"),
            )
            .await
            .expect("request should complete");
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body should be read");

        assert_eq!(
            (
                status,
                serde_json::from_slice::<serde_json::Value>(&body)
                    .expect("response should be JSON"),
            ),
            (
                StatusCode::OK,
                serde_json::json!({
                    "id": "review-z-older",
                    "frontmatter": null,
                    "body": "Earlier body.",
                }),
            ),
        );
    }

    #[tokio::test]
    async fn returns_not_found_when_a_review_marker_has_no_live_lock() {
        let temp_dir = TempDir::new().expect("fixture directory should be created");
        let document_path = temp_dir.path().join("stale-document.md");
        std::fs::write(&document_path, "Stale body.").expect("stale document should be written");
        let session = session_with_review(&document_path);
        let app = router_with_session_loader(move |_| Ok(Some(session.clone())));
        let response = app
            .oneshot(
                Request::get("/sessions/fixture-session/review-document")
                    .body(Body::empty())
                    .expect("request should be valid"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn skips_the_review_queue_when_the_session_has_no_document_marker() {
        let mut session = session_with_review(Path::new("/fixture/document.md"));
        session.pending_human_review_ids.clear();
        let queue_loader_calls = Arc::new(AtomicUsize::new(0));
        let queue_loader_calls_for_request = queue_loader_calls.clone();
        let app = router_with_loaders(
            move |_| Ok(Some(session.clone())),
            move |_| {
                queue_loader_calls_for_request.fetch_add(1, Ordering::Relaxed);
                ReviewDocumentQueue::Unmanaged
            },
        );
        let response = app
            .oneshot(
                Request::get("/sessions/fixture-session/review-document")
                    .body(Body::empty())
                    .expect("request should be valid"),
            )
            .await
            .expect("request should complete");

        assert_eq!(
            (
                response.status(),
                queue_loader_calls.load(Ordering::Relaxed),
            ),
            (StatusCode::NOT_FOUND, 0),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn saves_only_the_active_review_document_even_when_the_request_contains_a_path(
        active_review_document: ActiveReviewDocument,
    ) {
        let document_path = active_review_document.document_path;
        let outside_path = active_review_document
            .temp_dir
            .path()
            .join("outside-document.md");
        std::fs::write(
            &document_path,
            indoc! {"
                ---
                approved: false
                ---
                Old body."},
        )
        .expect("fixture document should be written");
        std::fs::write(&outside_path, "Keep this content.")
            .expect("outside document should be written");
        let session = active_review_document.session;
        let app = router_with_session_loader(move |_| Ok(Some(session.clone())));
        let response = app
            .oneshot(
                Request::post("/sessions/fixture-session/review-document/save")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "id": "fixture-review",
                            "path": outside_path,
                            "frontmatter": { "approved": true },
                            "body": "New body.",
                        })
                        .to_string(),
                    ))
                    .expect("request should be valid"),
            )
            .await
            .expect("request should complete");
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body should be read");

        assert_eq!(
            (
                status,
                serde_json::from_slice::<serde_json::Value>(&body)
                    .expect("response should be JSON"),
                std::fs::read_to_string(&document_path).expect("saved document should be readable"),
                std::fs::read_to_string(&outside_path)
                    .expect("outside document should be readable"),
            ),
            (
                StatusCode::OK,
                serde_json::json!({ "success": true }),
                indoc! {"
                    ---
                    approved: true
                    ---
                    New body."}
                .to_string(),
                "Keep this content.".to_string(),
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn saves_a_plain_markdown_document_without_adding_null_frontmatter(
        active_review_document: ActiveReviewDocument,
    ) {
        let document_path = active_review_document.document_path;
        let session = active_review_document.session;
        let app = router_with_session_loader(move |_| Ok(Some(session.clone())));
        let response = app
            .oneshot(
                Request::post("/sessions/fixture-session/review-document/save")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "id": "fixture-review",
                            "frontmatter": null,
                            "body": "Edited plain Markdown body.",
                        })
                        .to_string(),
                    ))
                    .expect("request should be valid"),
            )
            .await
            .expect("request should complete");

        assert_eq!(
            (
                response.status(),
                std::fs::read_to_string(&document_path).expect("saved document should be readable"),
            ),
            (StatusCode::OK, "Edited plain Markdown body.".to_string(),),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn rejects_a_stale_review_id_without_overwriting_the_document(
        active_review_document: ActiveReviewDocument,
    ) {
        let document_path = active_review_document.document_path;
        let session = active_review_document.session;
        let app = router_with_session_loader(move |_| Ok(Some(session.clone())));
        let response = app
            .oneshot(
                Request::post("/sessions/fixture-session/review-document/save")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "id": "another-review",
                            "frontmatter": null,
                            "body": "Must not be saved.",
                        })
                        .to_string(),
                    ))
                    .expect("request should be valid"),
            )
            .await
            .expect("request should complete");

        assert_eq!(
            (
                response.status(),
                std::fs::read_to_string(&document_path)
                    .expect("fixture document should be readable"),
            ),
            (StatusCode::CONFLICT, "Original body.".to_string()),
        );
    }

    #[cfg(unix)]
    #[rstest]
    #[tokio::test]
    async fn finish_saves_then_signals_review_process_without_signaling_done_fifo(
        active_review_document: ActiveReviewDocument,
    ) {
        use std::io::Read;
        use std::os::unix::fs::OpenOptionsExt;

        let document_path = active_review_document.document_path;
        let finish_fifo_path = finish_request_fifo_path(&document_path);
        let finish_ack_path = finish_ack_fifo_path(&document_path);
        let mut done_fifo_name = document_path.as_os_str().to_os_string();
        done_fifo_name.push(".done");
        let done_fifo_path = std::path::PathBuf::from(done_fifo_name);
        create_fixture_fifo(&done_fifo_path);
        let mut finish_reader = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&finish_fifo_path)
            .expect("finish request FIFO reader should be opened");
        let mut done_reader = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&done_fifo_path)
            .expect("done FIFO reader should be opened");
        let mut session = active_review_document.session;
        session.pending_human_review_ids = BTreeSet::from([review_marker(
            "fixture-review",
            &document_path,
            1,
            Some("fixture-editor-session"),
        )]);
        let app = router_with_loaders(
            move |_| Ok(Some(session.clone())),
            |_| ReviewDocumentQueue::Managed(Some("fixture-editor-session".to_string())),
        );
        let request = Request::post("/sessions/fixture-session/review-document/finish")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "id": "fixture-review",
                    "frontmatter": null,
                    "body": "Saved before review closes.",
                })
                .to_string(),
            ))
            .expect("request should be valid");
        let finish_task = tokio::spawn(async move { app.oneshot(request).await });
        let start = std::time::Instant::now();
        let mut finish_signal = [0_u8; 1];
        let finish_bytes_read = loop {
            let bytes_read = finish_reader
                .read(&mut finish_signal)
                .expect("finish request FIFO should be readable");
            if bytes_read > 0 || start.elapsed() >= std::time::Duration::from_secs(2) {
                break bytes_read;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        };
        let document_when_signaled = std::fs::read_to_string(&document_path)
            .expect("saved document should be readable before finish acknowledgement");
        if finish_bytes_read > 0 {
            let review_operation_lock = ReviewDocumentOperationLock::acquire(&document_path)
                .expect("review process should acquire the shared edit lock");
            std::fs::remove_file(&finish_fifo_path)
                .expect("review process should close the finish FIFO");
            drop(review_operation_lock);
            crate::shared::human_in_the_loop::signal_fifo(&finish_ack_path);
        }
        let response = finish_task
            .await
            .expect("finish task should not panic")
            .expect("request should complete");
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body should be read");
        let mut done_signal = [0_u8; 1];
        let done_bytes_read = done_reader
            .read(&mut done_signal)
            .expect("finish should leave the done FIFO to the review process");

        assert_eq!(
            (
                status,
                serde_json::from_slice::<serde_json::Value>(&body)
                    .expect("response should be JSON"),
                std::fs::read_to_string(&document_path).expect("saved document should be readable"),
                document_when_signaled,
                finish_bytes_read,
                finish_signal,
                done_bytes_read,
                done_signal,
            ),
            (
                StatusCode::OK,
                serde_json::json!({ "success": true }),
                "Saved before review closes.".to_string(),
                "Saved before review closes.".to_string(),
                1,
                *b"0",
                0,
                [0],
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn finish_rejects_without_saving_after_review_closes_finish_fifo(
        active_review_document: ActiveReviewDocument,
    ) {
        let document_path = active_review_document.document_path;
        std::fs::remove_file(finish_request_fifo_path(&document_path))
            .expect("review process should close the finish FIFO before finishing");
        let session = active_review_document.session;
        let app = router_with_session_loader(move |_| Ok(Some(session.clone())));
        let response = app
            .oneshot(
                Request::post("/sessions/fixture-session/review-document/finish")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "id": "fixture-review",
                            "frontmatter": null,
                            "body": "Late finish should not overwrite this document.",
                        })
                        .to_string(),
                    ))
                    .expect("request should be valid"),
            )
            .await
            .expect("request should complete");

        assert_eq!(
            (
                response.status(),
                std::fs::read_to_string(&document_path)
                    .expect("fixture document should be readable"),
            ),
            (StatusCode::CONFLICT, "Original body.".to_string()),
        );
    }
}
