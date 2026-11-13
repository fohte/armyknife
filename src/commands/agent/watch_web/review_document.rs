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
use crate::shared::human_in_the_loop::{LockGuard, ReviewDocumentMarker, review_document_marker};

type SessionLoader = Arc<dyn Fn(&str) -> anyhow::Result<Option<Session>> + Send + Sync>;
type ReviewQueueLoader = Arc<dyn Fn(&Session) -> ReviewDocumentQueue + Send + Sync>;
type ReviewSessionKiller = Arc<dyn Fn(&str) -> anyhow::Result<()> + Send + Sync>;

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
    router_with_dependencies(
        store::load_session,
        review_document_queue,
        crate::infra::shpool::kill_session,
    )
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
    router_with_dependencies(load_session, load_review_queue, |_| Ok(()))
}

fn router_with_dependencies(
    load_session: impl Fn(&str) -> anyhow::Result<Option<Session>> + Send + Sync + 'static,
    load_review_queue: impl Fn(&Session) -> ReviewDocumentQueue + Send + Sync + 'static,
    kill_review_session: impl Fn(&str) -> anyhow::Result<()> + Send + Sync + 'static,
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
            Arc::new(kill_review_session) as ReviewSessionKiller,
        ))
}

async fn get_review_document(
    State((load_session, load_review_queue, _)): State<(
        SessionLoader,
        ReviewQueueLoader,
        ReviewSessionKiller,
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
    State((load_session, load_review_queue, _)): State<(
        SessionLoader,
        ReviewQueueLoader,
        ReviewSessionKiller,
    )>,
    RoutePath(session_id): RoutePath<String>,
    axum::Json(request): axum::Json<ReviewDocumentEditRequest>,
) -> Result<axum::Json<ReviewDocumentSaved>, StatusCode> {
    save_document_for_session(load_session, load_review_queue, session_id, request, false).await?;
    Ok(axum::Json(ReviewDocumentSaved { success: true }))
}

async fn finish_review_document(
    State((load_session, load_review_queue, kill_review_session)): State<(
        SessionLoader,
        ReviewQueueLoader,
        ReviewSessionKiller,
    )>,
    RoutePath(session_id): RoutePath<String>,
    axum::Json(request): axum::Json<ReviewDocumentEditRequest>,
) -> Result<axum::Json<ReviewDocumentSaved>, StatusCode> {
    let (fifo_path, queue_key) =
        save_document_for_session(load_session, load_review_queue, session_id, request, true)
            .await?;
    crate::shared::human_in_the_loop::signal_fifo(&fifo_path);
    if let Some(queue_key) = queue_key {
        tokio::task::spawn_blocking(move || kill_review_session(&queue_key))
            .await
            .map_err(|error| {
                tracing::error!(%error, "review editor shutdown task failed");
                StatusCode::INTERNAL_SERVER_ERROR
            })?
            .map_err(|error| {
                tracing::warn!(%error, "failed to close review editor session");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
    }
    Ok(axum::Json(ReviewDocumentSaved { success: true }))
}

async fn save_document_for_session(
    load_session: SessionLoader,
    load_review_queue: ReviewQueueLoader,
    session_id: String,
    request: ReviewDocumentEditRequest,
    finishing: bool,
) -> Result<(std::path::PathBuf, Option<String>), StatusCode> {
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
        if !LockGuard::is_locked(&marker.path) {
            return Err(StatusCode::CONFLICT);
        }
        let fifo_path = done_fifo_path(&marker.path);
        if finishing && !is_fifo(&fifo_path) {
            return Err(StatusCode::CONFLICT);
        }
        let content = render_review_document(&marker.path, request.frontmatter, &request.body)?;
        overwrite_review_document(&marker.path, content.as_bytes())?;
        Ok((fifo_path, marker.queue_key))
    })
    .await
    .map_err(|error| {
        tracing::error!(%error, "review document edit task failed");
        StatusCode::INTERNAL_SERVER_ERROR
    })?
}

fn overwrite_review_document(path: &std::path::Path, content: &[u8]) -> Result<(), StatusCode> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        tracing::warn!(%error, "failed to inspect active review document before saving");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    if !metadata.file_type().is_file() {
        return Err(StatusCode::CONFLICT);
    }
    let mut options = std::fs::OpenOptions::new();
    options
        .write(true)
        .truncate(true)
        .custom_flags(libc::O_NOFOLLOW);
    let mut file = options.open(path).map_err(|error| {
        tracing::warn!(%error, "failed to open active review document for saving");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    file.write_all(content).map_err(|error| {
        tracing::warn!(%error, "failed to write active review document");
        StatusCode::INTERNAL_SERVER_ERROR
    })
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
    if crate::shared::yaml_frontmatter::split_frontmatter(&current).is_some() {
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

fn done_fifo_path(document_path: &std::path::Path) -> std::path::PathBuf {
    let mut path = document_path.as_os_str().to_os_string();
    path.push(".done");
    path.into()
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
    let mut markers = session
        .pending_human_review_ids
        .iter()
        .filter_map(|review_id| review_document_marker(review_id))
        .filter(|marker| LockGuard::is_locked(&marker.path))
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
    use tempfile::TempDir;
    use tower::ServiceExt;

    use crate::commands::agent::review_queue::ReviewDocumentQueue;
    use crate::commands::agent::types::{Engine, Session, SessionStatus};
    use crate::shared::human_in_the_loop::LockGuard;

    use super::{router_with_dependencies, router_with_loaders, router_with_session_loader};

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

    #[tokio::test]
    async fn saves_only_the_active_review_document_even_when_the_request_contains_a_path() {
        let temp_dir = TempDir::new().expect("fixture directory should be created");
        let document_path = temp_dir.path().join("fixture-document.md");
        let outside_path = temp_dir.path().join("outside-document.md");
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
        let session = session_with_review(&document_path);
        let _review_lock =
            LockGuard::acquire(&document_path).expect("fixture review lock should be acquired");
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

    #[tokio::test]
    async fn rejects_a_stale_review_id_without_overwriting_the_document() {
        let temp_dir = TempDir::new().expect("fixture directory should be created");
        let document_path = temp_dir.path().join("fixture-document.md");
        std::fs::write(&document_path, "Original body.")
            .expect("fixture document should be written");
        let session = session_with_review(&document_path);
        let _review_lock =
            LockGuard::acquire(&document_path).expect("fixture review lock should be acquired");
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
    #[tokio::test]
    async fn finish_saves_then_signals_and_kills_the_active_editor_session() {
        use std::io::Read;
        use std::os::unix::fs::OpenOptionsExt;

        let temp_dir = TempDir::new().expect("fixture directory should be created");
        let document_path = temp_dir.path().join("fixture-document.md");
        let fifo_path = super::done_fifo_path(&document_path);
        std::fs::write(&document_path, "Old body.").expect("fixture document should be written");
        let fifo_name = std::ffi::CString::new(fifo_path.as_os_str().as_encoded_bytes())
            .expect("fixture FIFO path should not contain NUL");
        // SAFETY: The path is NUL-terminated and points inside this test's temporary directory.
        let fifo_result = unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o600) };
        assert_eq!(fifo_result, 0, "fixture FIFO should be created");
        let mut fifo_reader = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&fifo_path)
            .expect("fixture FIFO reader should be opened");
        let mut session = session_with_review(&document_path);
        session.pending_human_review_ids = BTreeSet::from([review_marker(
            "fixture-review",
            &document_path,
            1,
            Some("fixture-editor-session"),
        )]);
        let _review_lock =
            LockGuard::acquire(&document_path).expect("fixture review lock should be acquired");
        let killed_sessions = Arc::new(std::sync::Mutex::new(Vec::new()));
        let killed_sessions_for_request = killed_sessions.clone();
        let app = router_with_dependencies(
            move |_| Ok(Some(session.clone())),
            |_| ReviewDocumentQueue::Managed(Some("fixture-editor-session".to_string())),
            move |session| {
                killed_sessions_for_request
                    .lock()
                    .map_err(|error| anyhow::anyhow!(error.to_string()))?
                    .push(session.to_string());
                Ok(())
            },
        );
        let response = app
            .oneshot(
                Request::post("/sessions/fixture-session/review-document/finish")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "id": "fixture-review",
                            "frontmatter": null,
                            "body": "Saved before review closes.",
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
        let mut signal = [0_u8; 1];
        let bytes_read = fifo_reader
            .read(&mut signal)
            .expect("finish should write to the FIFO");
        let killed_sessions = killed_sessions
            .lock()
            .expect("session kill calls should be readable")
            .clone();

        assert_eq!(
            (
                status,
                serde_json::from_slice::<serde_json::Value>(&body)
                    .expect("response should be JSON"),
                std::fs::read_to_string(&document_path).expect("saved document should be readable"),
                bytes_read,
                signal,
                killed_sessions,
            ),
            (
                StatusCode::OK,
                serde_json::json!({ "success": true }),
                "Saved before review closes.".to_string(),
                1,
                *b"0",
                vec!["fixture-editor-session".to_string()],
            ),
        );
    }
}
