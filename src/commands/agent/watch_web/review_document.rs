use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{DefaultBodyLimit, Path as RoutePath, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::commands::agent::review_queue::{ReviewDocumentQueue, review_document_queue};
use crate::commands::agent::store;
use crate::commands::agent::types::Session;
#[cfg(test)]
use crate::shared::human_in_the_loop::ReviewDocumentOperationLock;
use crate::shared::human_in_the_loop::{
    LockGuard, ReviewDocumentMarker, finish_ack_fifo_path, finish_request_fifo_path,
    review_document_marker,
};

mod execute;
mod metadata;
mod save;
use metadata::{ReviewDocumentRoots, ReviewDocumentSummary, summarize};

type SessionLoader = Arc<dyn Fn(&str) -> anyhow::Result<Option<Session>> + Send + Sync>;
type ReviewQueueLoader = Arc<dyn Fn(&Session) -> ReviewDocumentQueue + Send + Sync>;
type ReviewDocumentEditLock = Arc<tokio::sync::Mutex<()>>;
type ReviewDocumentActionRegistry = Arc<tokio::sync::Mutex<ReviewDocumentActionRegistryState>>;

struct ReviewDocumentActionRegistryState {
    records: HashMap<(String, String), execute::ActionRecord>,
    executor: execute::ActionExecutor,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewDocumentResponse {
    revision: String,
    frontmatter: serde_json::Value,
    body: String,
    #[serde(flatten)]
    summary: ReviewDocumentSummary,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewDocumentEditRequest {
    id: String,
    revision: String,
    frontmatter: serde_json::Value,
    body: String,
}

#[derive(Serialize)]
struct ReviewDocumentSaved {
    success: bool,
    revision: String,
}

pub(super) fn router() -> Router {
    router_with_dependencies(
        store::load_session,
        review_document_queue,
        ReviewDocumentRoots::default(),
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
    router_with_dependencies(
        load_session,
        load_review_queue,
        ReviewDocumentRoots::default(),
    )
}

#[cfg(test)]
fn router_with_path_roots(
    load_session: impl Fn(&str) -> anyhow::Result<Option<Session>> + Send + Sync + 'static,
    load_review_queue: impl Fn(&Session) -> ReviewDocumentQueue + Send + Sync + 'static,
    roots: ReviewDocumentRoots,
) -> Router {
    router_with_dependencies(load_session, load_review_queue, roots)
}

fn router_with_dependencies(
    load_session: impl Fn(&str) -> anyhow::Result<Option<Session>> + Send + Sync + 'static,
    load_review_queue: impl Fn(&Session) -> ReviewDocumentQueue + Send + Sync + 'static,
    roots: ReviewDocumentRoots,
) -> Router {
    router_with_action_executor(
        load_session,
        load_review_queue,
        roots,
        Arc::new(|record| Box::pin(async move { execute::run_action(&record).await })),
    )
}

#[cfg(test)]
fn router_with_injected_action_executor(
    load_session: impl Fn(&str) -> anyhow::Result<Option<Session>> + Send + Sync + 'static,
    load_review_queue: impl Fn(&Session) -> ReviewDocumentQueue + Send + Sync + 'static,
    roots: ReviewDocumentRoots,
    executor: execute::ActionExecutor,
) -> Router {
    router_with_action_executor(load_session, load_review_queue, roots, executor)
}

fn router_with_action_executor(
    load_session: impl Fn(&str) -> anyhow::Result<Option<Session>> + Send + Sync + 'static,
    load_review_queue: impl Fn(&Session) -> ReviewDocumentQueue + Send + Sync + 'static,
    roots: ReviewDocumentRoots,
    executor: execute::ActionExecutor,
) -> Router {
    Router::new()
        .route("/sessions/{id}/review-document", get(get_review_document))
        .route("/sessions/{id}/review-documents", get(get_review_documents))
        .route(
            "/sessions/{id}/review-documents/{document_id}",
            get(get_review_document_by_id),
        )
        .route(
            "/sessions/{id}/review-documents/{document_id}/save",
            post(save_review_document_by_id),
        )
        .route(
            "/sessions/{id}/review-documents/{document_id}/finish",
            post(finish_review_document_by_id),
        )
        .route(
            "/sessions/{id}/review-document/save",
            post(save_review_document),
        )
        .route(
            "/sessions/{id}/review-document/finish",
            post(finish_review_document),
        )
        .route(
            "/sessions/{id}/review-document/action",
            post(execute::execute_review_document),
        )
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
        .with_state((
            Arc::new(load_session) as SessionLoader,
            Arc::new(load_review_queue) as ReviewQueueLoader,
            Arc::new(tokio::sync::Mutex::new(())) as ReviewDocumentEditLock,
            roots,
            Arc::new(tokio::sync::Mutex::new(ReviewDocumentActionRegistryState {
                records: HashMap::new(),
                executor,
            })) as ReviewDocumentActionRegistry,
        ))
}

async fn get_review_document(
    State((load_session, load_review_queue, _, roots, _)): State<(
        SessionLoader,
        ReviewQueueLoader,
        ReviewDocumentEditLock,
        ReviewDocumentRoots,
        ReviewDocumentActionRegistry,
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
        let Some(marker) = active_review_document(&session, queue.clone()) else {
            return Ok(None);
        };
        let summary = summary_for_marker(&session, &marker, queue.clone(), &roots);
        match load_review_document(&marker, summary) {
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

async fn get_review_documents(
    State((load_session, load_review_queue, _, roots, _)): State<(
        SessionLoader,
        ReviewQueueLoader,
        ReviewDocumentEditLock,
        ReviewDocumentRoots,
        ReviewDocumentActionRegistry,
    )>,
    RoutePath(session_id): RoutePath<String>,
) -> Result<Json<Vec<ReviewDocumentSummary>>, StatusCode> {
    let documents = tokio::task::spawn_blocking(move || {
        let session = match load_session(&session_id) {
            Ok(Some(session)) => session,
            Ok(None) => return Ok(None),
            Err(error) => {
                tracing::warn!(%error, "failed to load session for review document list");
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
        };
        let queue = load_review_queue(&session);
        let markers = review_document_markers(&session);
        let editor_id = active_review_document(&session, queue.clone()).map(|marker| marker.id);
        let mut summaries = markers
            .iter()
            .map(|marker| summary_for_editor(marker, editor_id.as_deref(), &roots, &session.cwd))
            .collect::<Vec<_>>();
        summaries.sort_by_key(|summary| std::cmp::Reverse(summary.is_editor));
        Ok(Some(summaries))
    })
    .await
    .map_err(|error| {
        tracing::error!(%error, "review document list task failed");
        StatusCode::INTERNAL_SERVER_ERROR
    })??;

    documents.map(Json).ok_or(StatusCode::NOT_FOUND)
}

async fn get_review_document_by_id(
    State((load_session, load_review_queue, _, roots, _)): State<(
        SessionLoader,
        ReviewQueueLoader,
        ReviewDocumentEditLock,
        ReviewDocumentRoots,
        ReviewDocumentActionRegistry,
    )>,
    RoutePath((session_id, document_id)): RoutePath<(String, String)>,
) -> Result<Json<ReviewDocumentResponse>, StatusCode> {
    let response = tokio::task::spawn_blocking(move || {
        let session = load_session(&session_id)
            .map_err(|error| {
                tracing::warn!(%error, "failed to load session for review document");
                StatusCode::INTERNAL_SERVER_ERROR
            })?
            .ok_or(StatusCode::NOT_FOUND)?;
        let queue = load_review_queue(&session);
        let Some(marker) = review_document_markers(&session)
            .into_iter()
            .find(|marker| marker.id == document_id)
        else {
            return Err(StatusCode::NOT_FOUND);
        };
        let summary = summary_for_marker(&session, &marker, queue.clone(), &roots);
        load_review_document(&marker, summary).map_err(|error| {
            tracing::warn!(%error, "failed to read review document");
            StatusCode::INTERNAL_SERVER_ERROR
        })
    })
    .await
    .map_err(|error| {
        tracing::error!(%error, "review document task failed");
        StatusCode::INTERNAL_SERVER_ERROR
    })??;

    Ok(Json(response))
}

async fn save_review_document(
    State((load_session, load_review_queue, edit_lock, _, _)): State<(
        SessionLoader,
        ReviewQueueLoader,
        ReviewDocumentEditLock,
        ReviewDocumentRoots,
        ReviewDocumentActionRegistry,
    )>,
    RoutePath(session_id): RoutePath<String>,
    axum::Json(request): axum::Json<ReviewDocumentEditRequest>,
) -> Result<axum::Json<ReviewDocumentSaved>, StatusCode> {
    let _edit_guard = edit_lock.lock().await;
    let revision =
        save::save_document_for_session(load_session, load_review_queue, session_id, request, None)
            .await?;
    Ok(axum::Json(ReviewDocumentSaved {
        success: true,
        revision,
    }))
}

async fn finish_review_document(
    State((load_session, load_review_queue, edit_lock, _, _)): State<(
        SessionLoader,
        ReviewQueueLoader,
        ReviewDocumentEditLock,
        ReviewDocumentRoots,
        ReviewDocumentActionRegistry,
    )>,
    RoutePath(session_id): RoutePath<String>,
    axum::Json(request): axum::Json<ReviewDocumentEditRequest>,
) -> Result<axum::Json<ReviewDocumentSaved>, StatusCode> {
    let _edit_guard = edit_lock.lock().await;
    let revision = save::save_document_for_session(
        load_session,
        load_review_queue,
        session_id,
        request,
        Some(b'0'),
    )
    .await?;
    Ok(axum::Json(ReviewDocumentSaved {
        success: true,
        revision,
    }))
}

async fn save_review_document_by_id(
    State((load_session, load_review_queue, edit_lock, _, _)): State<(
        SessionLoader,
        ReviewQueueLoader,
        ReviewDocumentEditLock,
        ReviewDocumentRoots,
        ReviewDocumentActionRegistry,
    )>,
    RoutePath((session_id, document_id)): RoutePath<(String, String)>,
    axum::Json(request): axum::Json<ReviewDocumentEditRequest>,
) -> Result<axum::Json<ReviewDocumentSaved>, StatusCode> {
    save_review_document_for_id(
        load_session,
        load_review_queue,
        edit_lock,
        session_id,
        document_id,
        request,
        None,
    )
    .await
}

async fn finish_review_document_by_id(
    State((load_session, load_review_queue, edit_lock, _, _)): State<(
        SessionLoader,
        ReviewQueueLoader,
        ReviewDocumentEditLock,
        ReviewDocumentRoots,
        ReviewDocumentActionRegistry,
    )>,
    RoutePath((session_id, document_id)): RoutePath<(String, String)>,
    axum::Json(request): axum::Json<ReviewDocumentEditRequest>,
) -> Result<axum::Json<ReviewDocumentSaved>, StatusCode> {
    save_review_document_for_id(
        load_session,
        load_review_queue,
        edit_lock,
        session_id,
        document_id,
        request,
        Some(b'0'),
    )
    .await
}

async fn save_review_document_for_id(
    load_session: SessionLoader,
    load_review_queue: ReviewQueueLoader,
    edit_lock: ReviewDocumentEditLock,
    session_id: String,
    document_id: String,
    request: ReviewDocumentEditRequest,
    finish_signal: Option<u8>,
) -> Result<axum::Json<ReviewDocumentSaved>, StatusCode> {
    if request.id != document_id {
        return Err(StatusCode::CONFLICT);
    }
    let _edit_guard = edit_lock.lock().await;
    let revision = save::save_document_for_session(
        load_session,
        load_review_queue,
        session_id,
        request,
        finish_signal,
    )
    .await?;
    Ok(axum::Json(ReviewDocumentSaved {
        success: true,
        revision,
    }))
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReviewDocumentSessionState {
    pub(super) count: usize,
    pub(super) editor_id: Option<String>,
}

pub(super) fn review_document_session_states(
    sessions: &[Session],
    load_review_queue: impl Fn(&Session) -> ReviewDocumentQueue,
) -> HashMap<String, ReviewDocumentSessionState> {
    sessions
        .iter()
        .filter_map(|session| {
            let markers = review_document_markers(session);
            if markers.is_empty() {
                return None;
            }
            let editor_id = active_review_document_id(session, load_review_queue(session));
            Some((
                session.session_id.clone(),
                ReviewDocumentSessionState {
                    count: markers.len(),
                    editor_id,
                },
            ))
        })
        .collect()
}

fn review_document_markers(session: &Session) -> Vec<ReviewDocumentMarker> {
    let mut markers = session
        .pending_human_review_ids
        .iter()
        .filter_map(|review_id| review_document_marker(review_id))
        .collect::<Vec<_>>();
    markers.sort_by(|left, right| {
        left.started_at
            .cmp(&right.started_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    markers
}

fn summary_for_marker(
    session: &Session,
    marker: &ReviewDocumentMarker,
    queue: ReviewDocumentQueue,
    roots: &ReviewDocumentRoots,
) -> ReviewDocumentSummary {
    let is_editor =
        active_review_document(session, queue).is_some_and(|active| active.id == marker.id);
    summary_for_editor(
        marker,
        is_editor.then_some(marker.id.as_str()),
        roots,
        &session.cwd,
    )
}

fn summary_for_editor(
    marker: &ReviewDocumentMarker,
    editor_id: Option<&str>,
    roots: &ReviewDocumentRoots,
    session_cwd: &std::path::Path,
) -> ReviewDocumentSummary {
    let is_editor = editor_id == Some(marker.id.as_str());
    let request_fifo = finish_request_fifo_path(&marker.path);
    let can_save = is_editor && save::is_fifo(&request_fifo);
    let can_finish = can_save && save::is_fifo(&finish_ack_fifo_path(&marker.path));
    summarize(roots, marker, session_cwd, is_editor, can_save, can_finish)
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
    let markers = review_document_markers(session);

    match queue {
        ReviewDocumentQueue::Managed(Some(shown_key)) => markers
            .into_iter()
            .find(|marker| marker.queue_key.as_deref() == Some(shown_key.as_str())),
        ReviewDocumentQueue::Managed(None) | ReviewDocumentQueue::Unavailable => None,
        ReviewDocumentQueue::Unmanaged => markers
            .into_iter()
            .find(|marker| marker.queue_key.is_none()),
    }
}

fn load_review_document(
    marker: &ReviewDocumentMarker,
    summary: ReviewDocumentSummary,
) -> anyhow::Result<ReviewDocumentResponse> {
    let content = std::fs::read_to_string(&marker.path)?;
    let revision = review_document_revision(content.as_bytes());
    let (frontmatter, body) = match crate::shared::yaml_frontmatter::split_frontmatter(&content) {
        Some((_, yaml, body_offset)) => (
            serde_json::to_value(serde_yaml::from_str::<serde_yaml::Value>(yaml)?)?,
            content[body_offset..].to_owned(),
        ),
        None => (serde_json::Value::Null, content),
    };

    Ok(ReviewDocumentResponse {
        revision,
        frontmatter,
        body,
        summary,
    })
}

fn review_document_revision(content: &[u8]) -> String {
    crate::shared::hex::encode(&Sha256::digest(content))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

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
        ReviewDocumentOperationLock, ReviewDocumentRoots, finish_ack_fifo_path,
        finish_request_fifo_path, review_document_revision, router_with_injected_action_executor,
        router_with_loaders, router_with_path_roots, router_with_session_loader,
    };

    fn read_fifo_signal_or_pending(
        reader: &mut std::fs::File,
        signal: &mut [u8],
    ) -> std::io::Result<usize> {
        use std::io::Read;

        reader.read(signal).or_else(|error| {
            if error.kind() == std::io::ErrorKind::WouldBlock {
                Ok(0)
            } else {
                Err(error)
            }
        })
    }

    fn current_revision(path: &Path) -> String {
        let content = std::fs::read(path).expect("fixture document should be readable");
        review_document_revision(&content)
    }

    async fn wait_for_file(path: &Path, timeout: Duration) -> bool {
        let started_at = std::time::Instant::now();
        while !path.exists() {
            if started_at.elapsed() >= timeout {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        true
    }

    fn terminate_test_worker(worker: &mut std::process::Child) {
        if let Ok(process_group) = i32::try_from(worker.id()) {
            // SAFETY: the worker owns a dedicated process group for this test.
            let _ = unsafe { libc::kill(-process_group, libc::SIGKILL) };
        }
        let _ = worker.kill();
        let _ = worker.wait();
    }

    async fn wait_for_test_worker(worker: &mut std::process::Child) -> bool {
        let started_at = std::time::Instant::now();
        loop {
            if let Some(status) = worker
                .try_wait()
                .expect("test worker should be inspectable")
            {
                return status.success();
            }
            if started_at.elapsed() >= Duration::from_secs(15) {
                terminate_test_worker(worker);
                return false;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

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
            launch_id: None,
            session_id: "fixture-session".to_string(),
            archived_at: None,
            pr_context: None,
            created_pr_urls: Vec::new(),
            crit_urls: Vec::new(),
            crit_not_waiting_urls: Default::default(),
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
            last_prompted_at: None,
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
                    "revision": current_revision(&document_path),
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
                    "kind": "draft",
                    "target": { "path": "fixture-document.md" },
                    "isEditor": true,
                    "canSave": false,
                    "canFinish": false,
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
                    "revision": current_revision(&visible_path),
                    "frontmatter": null,
                    "body": "Visible body.",
                    "kind": "draft",
                    "target": { "path": "visible-document.md" },
                    "isEditor": true,
                    "canSave": false,
                    "canFinish": false,
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
                    "revision": current_revision(&older_path),
                    "frontmatter": null,
                    "body": "Earlier body.",
                    "kind": "draft",
                    "target": { "path": "older-document.md" },
                    "isEditor": true,
                    "canSave": false,
                    "canFinish": false,
                }),
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn lists_the_editor_document_before_queued_documents(
        active_review_document: ActiveReviewDocument,
    ) {
        let queued_path = active_review_document
            .temp_dir
            .path()
            .join("queued-document.md");
        std::fs::write(&queued_path, "Queued body.").expect("queued document should be written");
        let mut session = active_review_document.session;
        session.pending_human_review_ids.insert(review_marker(
            "queued-review",
            &queued_path,
            2,
            None,
        ));
        let app = router_with_session_loader(move |_| Ok(Some(session.clone())));
        let response = app
            .oneshot(
                Request::get("/sessions/fixture-session/review-documents")
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
                serde_json::json!([
                    {
                        "id": "fixture-review",
                        "kind": "draft",
                        "target": { "path": "fixture-document.md" },
                        "isEditor": true,
                        "canSave": true,
                        "canFinish": true,
                    },
                    {
                        "id": "queued-review",
                        "kind": "draft",
                        "target": { "path": "queued-document.md" },
                        "isEditor": false,
                        "canSave": false,
                        "canFinish": false,
                    },
                ]),
            ),
        );
    }

    #[tokio::test]
    async fn lists_typed_documents_with_their_targets_through_the_review_api() {
        let temp_dir = TempDir::new().expect("fixture directory should be created");
        let roots = ReviewDocumentRoots {
            pr_draft: temp_dir.path().join("pr-body-draft"),
            issue_agent: temp_dir.path().join("gh-issue-agent"),
            pr_review: temp_dir.path().join("gh-pr-review"),
        };
        #[cfg(unix)]
        let pr_draft_root = {
            use std::os::unix::fs::symlink;

            let actual_root = temp_dir.path().join("actual-pr-body-draft");
            std::fs::create_dir_all(&actual_root).expect("actual draft root should be created");
            symlink(&actual_root, &roots.pr_draft).expect("draft root symlink should be created");
            actual_root
        };
        #[cfg(not(unix))]
        let pr_draft_root = roots.pr_draft.clone();
        let pr_draft_path = pr_draft_root.join("sample-owner/sample-repo/feature/example.md");
        let issue_path = roots
            .issue_agent
            .join("sample-owner/sample-repo/7319/issue.md");
        let comment_path = roots
            .issue_agent
            .join("sample-owner/sample-repo/7319/comments/new_reply.md");
        let existing_comment_path = roots
            .issue_agent
            .join("sample-owner/sample-repo/7319/comments/001_comment_88600.md");
        let legacy_issue_path = roots
            .issue_agent
            .join("sample-owner/sample-repo/5843/issue.md");
        let legacy_metadata_path = legacy_issue_path
            .parent()
            .expect("legacy issue should have a parent")
            .join("metadata.json");
        let legacy_comment_path = roots
            .issue_agent
            .join("sample-owner/sample-repo/5843/comments/002_comment_88601.md");
        let new_issue_path = roots
            .issue_agent
            .join("sample-owner/sample-repo/new/issue.md");
        let replies_path = roots
            .pr_review
            .join("sample-owner/sample-repo/8462/threads.md");
        let generic_path = temp_dir.path().join("docs/retry-plan.md");
        for (path, contents) in [
            (
                pr_draft_path.as_path(),
                indoc! {"
                    ---
                    title: Sample pull request
                    ---
                    Body.
                "},
            ),
            (
                issue_path.as_path(),
                indoc! {"
                    ---
                    title: Sample issue
                    readonly:
                      number: 7319
                    ---
                    Body.
                "},
            ),
            (comment_path.as_path(), "Comment body."),
            (existing_comment_path.as_path(), "Existing comment body."),
            (legacy_issue_path.as_path(), "Legacy issue body."),
            (
                legacy_metadata_path.as_path(),
                indoc! {r#"
                    {
                      "number": 5843,
                      "title": "Legacy sample issue",
                      "state": "OPEN",
                      "labels": [],
                      "assignees": [],
                      "milestone": null,
                      "author": "sample-author",
                      "createdAt": "2026-01-01T00:00:00Z",
                      "updatedAt": "2026-01-02T00:00:00Z"
                    }
                "#},
            ),
            (legacy_comment_path.as_path(), "Legacy comment body."),
            (
                new_issue_path.as_path(),
                indoc! {"
                    ---
                    title: New issue
                    ---
                    Body.
                "},
            ),
            (
                replies_path.as_path(),
                indoc! {r#"
                    ---
                    pr: 8462
                    repo: sample-owner/sample-repo
                    pulled_at: 2026-01-15T10:00:00Z
                    ---

                    <!-- reviews -->
                    <!-- /reviews -->

                    <!-- thread: RT_sample path: src/lib.rs:7 -->
                    - [ ] resolve
                    <!-- diff -->
                    ```diff
                    @@ -1 +1 @@
                    -old
                    +new
                    ```
                    <!-- /diff -->
                    <!-- comment: @reviewer 2026-01-15T10:30:00Z -->
                    Sample comment.
                    <!-- /comment -->
                "#},
            ),
            (generic_path.as_path(), "Generic body."),
        ] {
            std::fs::create_dir_all(path.parent().expect("fixture file should have a parent"))
                .expect("fixture directory should be created");
            std::fs::write(path, contents).expect("fixture document should be written");
        }

        let mut session = session_with_review(&generic_path);
        session.cwd = temp_dir.path().to_path_buf();
        session.pending_human_review_ids.clear();
        for (id, path, started_at) in [
            ("pr-draft", &pr_draft_path, 1),
            ("issue-body", &issue_path, 2),
            ("issue-comment", &comment_path, 3),
            ("existing-issue-comment", &existing_comment_path, 4),
            ("legacy-issue-body", &legacy_issue_path, 5),
            ("legacy-issue-comment", &legacy_comment_path, 6),
            ("new-issue", &new_issue_path, 7),
            ("review-replies", &replies_path, 8),
            ("generic", &generic_path, 1_800_000_123_456_000_000),
        ] {
            session
                .pending_human_review_ids
                .insert(review_marker(id, path, started_at, None));
        }
        let app = router_with_path_roots(
            move |_| Ok(Some(session.clone())),
            |_| ReviewDocumentQueue::Unmanaged,
            roots,
        );
        let response = app
            .oneshot(
                Request::get("/sessions/fixture-session/review-documents")
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
                serde_json::json!([
                    {
                        "id": "pr-draft",
                        "kind": "pr_draft",
                        "target": {
                            "repository": "sample-owner/sample-repo",
                            "branch": "feature/example",
                            "title": "Sample pull request",
                        },
                        "isEditor": false,
                        "canSave": false,
                        "canFinish": false,
                    },
                    {
                        "id": "issue-body",
                        "kind": "issue_body",
                        "target": {
                            "repository": "sample-owner/sample-repo",
                            "issueNumber": 7319,
                            "title": "Sample issue",
                        },
                        "isEditor": false,
                        "canSave": false,
                        "canFinish": false,
                    },
                    {
                        "id": "issue-comment",
                        "kind": "issue_comment",
                        "target": {
                            "repository": "sample-owner/sample-repo",
                            "issueNumber": 7319,
                            "title": "Sample issue",
                        },
                        "isEditor": false,
                        "canSave": false,
                        "canFinish": false,
                    },
                    {
                        "id": "existing-issue-comment",
                        "kind": "issue_comment",
                        "target": {
                            "repository": "sample-owner/sample-repo",
                            "issueNumber": 7319,
                            "title": "Sample issue",
                        },
                        "isEditor": false,
                        "canSave": false,
                        "canFinish": false,
                    },
                    {
                        "id": "legacy-issue-body",
                        "kind": "issue_body",
                        "target": {
                            "repository": "sample-owner/sample-repo",
                            "issueNumber": 5843,
                            "title": "Legacy sample issue",
                        },
                        "isEditor": false,
                        "canSave": false,
                        "canFinish": false,
                    },
                    {
                        "id": "legacy-issue-comment",
                        "kind": "issue_comment",
                        "target": {
                            "repository": "sample-owner/sample-repo",
                            "issueNumber": 5843,
                            "title": "Legacy sample issue",
                        },
                        "isEditor": false,
                        "canSave": false,
                        "canFinish": false,
                    },
                    {
                        "id": "new-issue",
                        "kind": "draft",
                        "target": {
                            "repository": "sample-owner/sample-repo",
                            "title": "New issue",
                        },
                        "isEditor": false,
                        "canSave": false,
                        "canFinish": false,
                    },
                    {
                        "id": "review-replies",
                        "kind": "review_replies",
                        "target": {
                            "repository": "sample-owner/sample-repo",
                            "pullRequestNumber": 8462,
                            "threadCount": 1,
                        },
                        "isEditor": false,
                        "canSave": false,
                        "canFinish": false,
                    },
                    {
                        "id": "generic",
                        "kind": "draft",
                        "target": { "path": "docs/retry-plan.md" },
                        "startedAt": 1_800_000_123_456_u64,
                        "isEditor": false,
                        "canSave": false,
                        "canFinish": false,
                    },
                ]),
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn gets_a_queued_document_by_id(active_review_document: ActiveReviewDocument) {
        let queued_path = active_review_document
            .temp_dir
            .path()
            .join("queued-document.md");
        std::fs::write(&queued_path, "Queued body.").expect("queued document should be written");
        let mut session = active_review_document.session;
        session.pending_human_review_ids.insert(review_marker(
            "queued-review",
            &queued_path,
            2,
            None,
        ));
        let app = router_with_session_loader(move |_| Ok(Some(session.clone())));
        let response = app
            .oneshot(
                Request::get("/sessions/fixture-session/review-documents/queued-review")
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
                    "id": "queued-review",
                    "revision": current_revision(&queued_path),
                    "frontmatter": null,
                    "body": "Queued body.",
                    "kind": "draft",
                    "target": { "path": "queued-document.md" },
                    "isEditor": false,
                    "canSave": false,
                    "canFinish": false,
                }),
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn rejects_saving_a_queued_document_by_id(active_review_document: ActiveReviewDocument) {
        let queued_path = active_review_document
            .temp_dir
            .path()
            .join("queued-document.md");
        std::fs::write(&queued_path, "Queued body.").expect("queued document should be written");
        let mut session = active_review_document.session;
        session.pending_human_review_ids.insert(review_marker(
            "queued-review",
            &queued_path,
            2,
            None,
        ));
        let app = router_with_session_loader(move |_| Ok(Some(session.clone())));
        let response = app
            .oneshot(
                Request::post("/sessions/fixture-session/review-documents/queued-review/save")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "id": "queued-review",
                            "revision": current_revision(&queued_path),
                            "frontmatter": null,
                            "body": "Changed body.",
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
                std::fs::read_to_string(&queued_path).expect("queued document should be readable"),
            ),
            (StatusCode::CONFLICT, "Queued body.".to_string()),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn rejects_a_document_id_that_does_not_match_the_save_route(
        active_review_document: ActiveReviewDocument,
    ) {
        let active_path = active_review_document.document_path;
        let queued_path = active_review_document
            .temp_dir
            .path()
            .join("queued-document.md");
        std::fs::write(&queued_path, "Queued body.").expect("queued document should be written");
        let mut session = active_review_document.session;
        session.pending_human_review_ids.insert(review_marker(
            "queued-review",
            &queued_path,
            2,
            None,
        ));
        let app = router_with_session_loader(move |_| Ok(Some(session.clone())));
        let response = app
            .oneshot(
                Request::post("/sessions/fixture-session/review-documents/queued-review/save")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "id": "fixture-review",
                            "revision": current_revision(&active_path),
                            "frontmatter": null,
                            "body": "Must not overwrite the active document.",
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
                std::fs::read_to_string(&active_path).expect("active document should be readable"),
                std::fs::read_to_string(&queued_path).expect("queued document should be readable"),
            ),
            (
                StatusCode::CONFLICT,
                "Original body.".to_string(),
                "Queued body.".to_string(),
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
                Request::post("/sessions/fixture-session/review-documents/fixture-review/save")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "id": "fixture-review",
                            "path": outside_path,
                            "revision": current_revision(&document_path),
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
                serde_json::json!({
                    "success": true,
                    "revision": current_revision(&document_path),
                }),
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
                            "revision": current_revision(&document_path),
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
                            "revision": current_revision(&document_path),
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

    #[rstest]
    #[case::save("save")]
    #[case::finish("finish")]
    #[tokio::test]
    async fn rejects_a_stale_revision_without_overwriting_the_document(
        active_review_document: ActiveReviewDocument,
        #[case] action: &str,
    ) {
        let document_path = active_review_document.document_path;
        let revision = current_revision(&document_path);
        std::fs::write(&document_path, "Latest body from another editor.")
            .expect("latest document should be written");
        let session = active_review_document.session;
        let app = router_with_session_loader(move |_| Ok(Some(session.clone())));
        let response = app
            .oneshot(
                Request::post(format!(
                    "/sessions/fixture-session/review-document/{action}"
                ))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "id": "fixture-review",
                        "revision": revision,
                        "frontmatter": null,
                        "body": "Stale client content.",
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
            (
                StatusCode::CONFLICT,
                "Latest body from another editor.".to_string(),
            ),
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn direct_action_uses_the_injected_executor_once_for_a_successful_document() {
        use std::os::unix::fs::OpenOptionsExt;

        crate::shared::testing::init_approval_dir();
        let temp_dir = TempDir::new().expect("fixture directory should be created");
        let roots = ReviewDocumentRoots {
            pr_draft: temp_dir.path().join("pr-body-draft"),
            issue_agent: temp_dir.path().join("gh-issue-agent"),
            pr_review: temp_dir.path().join("gh-pr-review"),
        };
        let document_path = roots
            .issue_agent
            .join("sample-owner/sample-repo/7319/comments/new_example.md");
        std::fs::create_dir_all(document_path.parent().expect("comment has a parent"))
            .expect("comment directory should be created");
        std::fs::write(
            &document_path,
            indoc! {"
                ---
                submit: false
                ---

                Example comment.
            "},
        )
        .expect("comment document should be written");
        create_fixture_fifo(&finish_request_fifo_path(&document_path));
        create_fixture_fifo(&finish_ack_fifo_path(&document_path));
        let review_lock = LockGuard::acquire(&document_path)
            .expect("review lock should be acquired for the fixture document");
        let mut session = session_with_review(&document_path);
        session.cwd = temp_dir.path().to_path_buf();
        let calls = Arc::new(AtomicUsize::new(0));
        let executor_calls = calls.clone();
        let action_started = Arc::new(tokio::sync::Notify::new());
        let executor_started = action_started.clone();
        let release_action = Arc::new(tokio::sync::Notify::new());
        let executor_release = release_action.clone();
        let app = router_with_injected_action_executor(
            move |_| Ok(Some(session.clone())),
            |_| ReviewDocumentQueue::Unmanaged,
            roots,
            Arc::new(move |_| {
                let calls = executor_calls.clone();
                let started = executor_started.clone();
                let release = executor_release.clone();
                Box::pin(async move {
                    let attempt = calls.fetch_add(1, Ordering::SeqCst);
                    if attempt == 0 {
                        started.notify_one();
                        release.notified().await;
                        Err(anyhow::anyhow!("Temporary submission failure"))
                    } else {
                        Ok("Pull request replies posted.".to_owned())
                    }
                })
            }),
        );
        let finish_fifo = finish_request_fifo_path(&document_path);
        let finish_ack_fifo = finish_ack_fifo_path(&document_path);
        let mut finish_reader = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&finish_fifo)
            .expect("finish request FIFO reader should be opened");
        let revision = current_revision(&document_path);
        let request_body = serde_json::json!({
            "id": "fixture-review",
            "revision": revision,
            "frontmatter": {"submit": false},
            "body": "Example comment.",
            "action": "post_comment",
        });
        let first_request = Request::post("/sessions/fixture-session/review-document/action")
            .header("content-type", "application/json")
            .body(Body::from(request_body.to_string()))
            .expect("action request should be valid");
        let first_app = app.clone();
        let first_task = tokio::spawn(async move { first_app.oneshot(first_request).await });
        let started_at = std::time::Instant::now();
        let mut signal = [0_u8; 1];
        let signal_bytes = loop {
            let bytes = read_fifo_signal_or_pending(&mut finish_reader, &mut signal)
                .expect("finish request FIFO read should succeed");
            if bytes > 0 || started_at.elapsed() >= Duration::from_secs(2) {
                break bytes;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        if signal_bytes > 0 {
            let operation_lock = ReviewDocumentOperationLock::acquire(&document_path)
                .expect("review process should acquire the operation lock");
            std::fs::remove_file(&finish_fifo)
                .expect("review process should close the finish request FIFO");
            drop(operation_lock);
            crate::shared::human_in_the_loop::signal_fifo_with_byte(&finish_ack_fifo, b'0');
        }
        drop(review_lock);
        tokio::time::timeout(Duration::from_secs(2), action_started.notified())
            .await
            .expect("injected action should start");
        let second_request = Request::post("/sessions/fixture-session/review-document/action")
            .header("content-type", "application/json")
            .body(Body::from(request_body.to_string()))
            .expect("concurrent duplicate action request should be valid");
        let second_app = app.clone();
        let mut second_task = tokio::spawn(async move { second_app.oneshot(second_request).await });
        let second_while_running =
            tokio::time::timeout(Duration::from_millis(100), &mut second_task).await;
        let returned_while_running = second_while_running.is_ok();
        release_action.notify_one();
        let second_response = match second_while_running {
            Ok(result) => result.expect("concurrent duplicate request should complete"),
            Err(_) => second_task
                .await
                .expect("queued duplicate request should not panic"),
        }
        .expect("concurrent duplicate request should complete");
        let second_status = second_response.status();
        let first_response = first_task
            .await
            .expect("first action task should not panic")
            .expect("first action request should complete");
        let first_status = first_response.status();
        let first_body = to_bytes(first_response.into_body(), usize::MAX)
            .await
            .expect("first response body should be read");
        let first_json = serde_json::from_slice::<serde_json::Value>(&first_body)
            .expect("first response should be JSON");
        let failed_document_hash = crate::shared::human_in_the_loop::document_hash(&document_path)
            .expect("failed document hash should be computed");
        let failed_action_result =
            crate::shared::human_in_the_loop::action_result_after_review_completion(
                b'1',
                &document_path,
            )
            .expect("failed action result should be readable")
            .expect("failed action result should be saved");
        let retry_response = app
            .clone()
            .oneshot(
                Request::post("/sessions/fixture-session/review-document/action")
                    .header("content-type", "application/json")
                    .body(Body::from(request_body.to_string()))
                    .expect("retry request should be valid"),
            )
            .await
            .expect("retry request should complete");
        let retry_status = retry_response.status();
        let retry_body = to_bytes(retry_response.into_body(), usize::MAX)
            .await
            .expect("retry response body should be read");
        let retry_json = serde_json::from_slice::<serde_json::Value>(&retry_body)
            .expect("retry response should be JSON");
        let third_response = app
            .oneshot(
                Request::post("/sessions/fixture-session/review-document/action")
                    .header("content-type", "application/json")
                    .body(Body::from(request_body.to_string()))
                    .expect("duplicate action request should be valid"),
            )
            .await
            .expect("completed action request should complete");
        let third_status = third_response.status();
        let third_body = to_bytes(third_response.into_body(), usize::MAX)
            .await
            .expect("completed action response body should be read");
        let third_json = serde_json::from_slice::<serde_json::Value>(&third_body)
            .expect("completed action response should be JSON");

        assert_eq!(
            (
                signal_bytes,
                signal[0],
                first_status,
                first_json,
                failed_action_result,
                returned_while_running,
                second_status,
                retry_status,
                retry_json,
                third_status,
                third_json,
                calls.load(Ordering::SeqCst),
            ),
            (
                1,
                b'1',
                StatusCode::OK,
                serde_json::json!({
                    "success": false,
                    "revision": current_revision(&document_path),
                    "error": "Temporary submission failure",
                }),
                crate::shared::human_in_the_loop::ReviewDocumentActionResult::new(
                    crate::shared::human_in_the_loop::ReviewDocumentAction::PostComment,
                    false,
                    "Temporary submission failure".to_owned(),
                    failed_document_hash.clone(),
                    Some(failed_document_hash),
                ),
                true,
                StatusCode::CONFLICT,
                StatusCode::OK,
                serde_json::json!({
                    "success": true,
                    "revision": current_revision(&document_path),
                }),
                StatusCode::OK,
                serde_json::json!({
                    "success": true,
                    "revision": current_revision(&document_path),
                }),
                2,
            ),
        );
    }

    #[cfg(unix)]
    #[rstest]
    #[case::pr_draft(
        "pr_draft",
        "create_pr",
        "owner/repository/topic.md",
        serde_json::json!({
            "title": "Example title",
            "steps": { "submit": false, "ready-for-translation": false }
        }),
        "https://example.invalid/pull/42"
    )]
    #[case::issue_comment(
        "issue_comment",
        "post_comment",
        "owner/repository/7319/comments/new_example.md",
        serde_json::json!({ "submit": false }),
        "Issue comment posted."
    )]
    #[case::pr_replies(
        "pr_replies",
        "post_replies",
        "owner/repository/42/threads.md",
        serde_json::json!({
            "pr": 42,
            "repo": "owner/repository",
            "pulled_at": "2026-01-01T00:00:00Z",
            "submit": false
        }),
        "Pull request replies posted."
    )]
    #[case::issue_body(
        "issue_body",
        "update_issue",
        "owner/repository/7319/issue.md",
        serde_json::json!({
            "title": "Example issue",
            "readonly": { "number": 7319 },
            "submit": false
        }),
        "Issue body updated."
    )]
    #[tokio::test]
    async fn web_action_result_is_saved_for_each_review_document(
        #[case] kind: &str,
        #[case] action: &str,
        #[case] relative_path: &str,
        #[case] frontmatter: serde_json::Value,
        #[case] result_message: &str,
    ) {
        use std::os::unix::fs::OpenOptionsExt;

        use crate::shared::human_in_the_loop::{
            ReviewDocumentAction, ReviewDocumentActionResult,
            action_result_after_review_completion, document_hash,
        };

        crate::shared::testing::init_approval_dir();
        let temp_dir = TempDir::new().expect("fixture directory should be created");
        let roots = ReviewDocumentRoots {
            pr_draft: temp_dir.path().join("pr-body-draft"),
            issue_agent: temp_dir.path().join("gh-issue-agent"),
            pr_review: temp_dir.path().join("gh-pr-review"),
        };
        let root = match kind {
            "pr_draft" => &roots.pr_draft,
            "issue_comment" | "issue_body" => &roots.issue_agent,
            "pr_replies" => &roots.pr_review,
            _ => unreachable!(),
        };
        let document_path = root.join(relative_path);
        std::fs::create_dir_all(document_path.parent().expect("document has a parent"))
            .expect("document directory should be created");
        std::fs::write(&document_path, "Fixture review document.")
            .expect("review document should be written");
        create_fixture_fifo(&finish_request_fifo_path(&document_path));
        create_fixture_fifo(&finish_ack_fifo_path(&document_path));
        let review_lock = LockGuard::acquire(&document_path)
            .expect("review lock should be acquired for the fixture document");
        let mut session = session_with_review(&document_path);
        session.cwd = temp_dir.path().to_path_buf();
        let executor_result_message = result_message.to_owned();
        let app = router_with_injected_action_executor(
            move |_| Ok(Some(session.clone())),
            |_| ReviewDocumentQueue::Unmanaged,
            roots,
            Arc::new(move |_| {
                let result_message = executor_result_message.clone();
                Box::pin(async move { Ok(result_message) })
            }),
        );
        let finish_fifo = finish_request_fifo_path(&document_path);
        let finish_ack_fifo = finish_ack_fifo_path(&document_path);
        let mut finish_reader = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&finish_fifo)
            .expect("finish request reader should be opened");
        let request_body = serde_json::json!({
            "id": "fixture-review",
            "revision": current_revision(&document_path),
            "frontmatter": frontmatter,
            "body": "Approved fixture document.",
            "action": action,
        });
        let request = Request::post("/sessions/fixture-session/review-document/action")
            .header("content-type", "application/json")
            .body(Body::from(request_body.to_string()))
            .expect("action request should be valid");
        let app = app.clone();
        let action_task = tokio::spawn(async move { app.oneshot(request).await });
        let started_at = std::time::Instant::now();
        let mut signal = [0_u8; 1];
        let signal_bytes = loop {
            let bytes = read_fifo_signal_or_pending(&mut finish_reader, &mut signal)
                .expect("finish FIFO read should succeed");
            if bytes > 0 || started_at.elapsed() >= Duration::from_secs(2) {
                break bytes;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        if signal_bytes > 0 {
            let operation_lock = ReviewDocumentOperationLock::acquire(&document_path)
                .expect("review process should acquire the operation lock");
            std::fs::remove_file(&finish_fifo).expect("finish FIFO should be removed");
            drop(operation_lock);
            crate::shared::human_in_the_loop::signal_fifo_with_byte(&finish_ack_fifo, b'0');
        }
        drop(review_lock);
        let response = action_task
            .await
            .expect("action task should not panic")
            .expect("action request should complete");
        let response_status = response.status();
        let response_body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("action response body should be read");
        let response_json = serde_json::from_slice::<serde_json::Value>(&response_body)
            .expect("action response should be JSON");
        let saved_hash = document_hash(&document_path).expect("saved document hash should exist");
        let saved_result = action_result_after_review_completion(b'1', &document_path)
            .expect("saved result should be readable");
        let action = match action {
            "create_pr" => ReviewDocumentAction::CreatePr,
            "post_comment" => ReviewDocumentAction::PostComment,
            "post_replies" => ReviewDocumentAction::PostReplies,
            "update_issue" => ReviewDocumentAction::UpdateIssue,
            _ => unreachable!(),
        };

        assert_eq!(
            (
                signal_bytes,
                signal[0],
                response_status,
                response_json,
                saved_result,
            ),
            (
                1,
                b'1',
                StatusCode::OK,
                serde_json::json!({
                    "success": true,
                    "revision": current_revision(&document_path),
                }),
                Some(ReviewDocumentActionResult::new(
                    action,
                    true,
                    result_message.to_owned(),
                    saved_hash.clone(),
                    Some(saved_hash),
                )),
            ),
        );
    }

    #[tokio::test]
    async fn web_action_waits_for_the_review_child_and_reports_its_result() {
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};
        use std::sync::Mutex;

        use crate::shared::human_in_the_loop::{
            ReviewDocumentAction, ReviewDocumentActionResult,
            action_result_after_review_completion, document_hash,
        };

        crate::shared::testing::init_approval_dir();
        let temp_dir = TempDir::new().expect("fixture directory should be created");
        let roots = ReviewDocumentRoots {
            pr_draft: temp_dir.path().join("pr-body-draft"),
            issue_agent: temp_dir.path().join("gh-issue-agent"),
            pr_review: temp_dir.path().join("gh-pr-review"),
        };
        let document_path = roots
            .issue_agent
            .join("owner/repository/7319/comments/comment.md");
        std::fs::create_dir_all(document_path.parent().expect("document has a parent"))
            .expect("document directory should be created");
        let original_document = indoc! {"
            ---
            submit: false
            ---
            Original comment.
        "};
        std::fs::write(&document_path, original_document)
            .expect("review document should be written");
        let ready_file = temp_dir.path().join("review-child-ready");
        let output_file = temp_dir.path().join("agent-output");
        let mut review_parent = Command::new(std::env::current_exe().expect("test binary path"))
            .args([
                "--exact",
                "shared::human_in_the_loop::tests::watch_web_review_parent_worker",
                "--ignored",
                "--nocapture",
            ])
            .env("ARMYKNIFE_REVIEW_TEST_DOCUMENT", &document_path)
            .env("ARMYKNIFE_REVIEW_TEST_READY_FILE", &ready_file)
            .env("ARMYKNIFE_REVIEW_TEST_OUTPUT_FILE", &output_file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .expect("review parent worker should start");
        let child_ready = wait_for_file(&ready_file, Duration::from_secs(5)).await;
        if !child_ready {
            terminate_test_worker(&mut review_parent);
        }
        assert!(
            child_ready,
            "review child should wait for the finish signal"
        );

        let mut session = session_with_review(&document_path);
        session.cwd = temp_dir.path().to_path_buf();
        let action_lock_state = Arc::new(Mutex::new(None));
        let executor_lock_state = action_lock_state.clone();
        let executor_path = document_path.clone();
        let app = router_with_injected_action_executor(
            move |_| Ok(Some(session.clone())),
            |_| ReviewDocumentQueue::Unmanaged,
            roots,
            Arc::new(move |_| {
                let competing_lock = LockGuard::acquire(&executor_path);
                let lock_state = (
                    LockGuard::is_locked(&executor_path),
                    competing_lock.is_err(),
                );
                drop(competing_lock);
                *executor_lock_state
                    .lock()
                    .expect("action lock state should be writable") = Some(lock_state);
                Box::pin(async { Ok("Issue comment posted.".to_owned()) })
            }),
        );
        let request = Request::post("/sessions/fixture-session/review-document/action")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "id": "fixture-review",
                    "revision": current_revision(&document_path),
                    "frontmatter": { "submit": false },
                    "body": "Approved fixture document.",
                    "action": "post_comment",
                })
                .to_string(),
            ))
            .expect("action request should be valid");
        let response = app
            .oneshot(request)
            .await
            .expect("action request should complete");
        let response_status = response.status();
        let response_body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("action response body should be readable");
        let response_json = serde_json::from_slice::<serde_json::Value>(&response_body)
            .expect("action response should be JSON");
        let mut action_result_file = document_path.as_os_str().to_os_string();
        action_result_file.push(".watch-web-action-result");
        let action_result_saved = PathBuf::from(action_result_file).exists();
        if !action_result_saved {
            terminate_test_worker(&mut review_parent);
        }
        let review_parent_succeeded = wait_for_test_worker(&mut review_parent).await;
        let printed_output = std::fs::read_to_string(&output_file)
            .expect("review process output should be captured");
        let action_result = if action_result_saved {
            action_result_after_review_completion(b'1', &document_path)
                .expect("action result should be readable")
        } else {
            None
        };
        let resulting_hash = document_hash(&document_path).expect("result hash should be read");
        let action_lock_state = action_lock_state
            .lock()
            .expect("action lock state should be readable")
            .take();

        assert_eq!(
            (
                child_ready,
                response_status,
                response_json,
                action_lock_state,
                action_result,
                review_parent_succeeded,
                printed_output,
            ),
            (
                true,
                StatusCode::OK,
                serde_json::json!({
                    "success": true,
                    "revision": current_revision(&document_path),
                }),
                Some((true, true)),
                Some(ReviewDocumentActionResult::new(
                    ReviewDocumentAction::PostComment,
                    true,
                    "Issue comment posted.".to_owned(),
                    resulting_hash.clone(),
                    Some(resulting_hash),
                )),
                true,
                indoc! {"
                    Watch-web issue comment succeeded: Issue comment posted.
                    No further submit or push command is needed.
                    @@ -1,4 +1,4 @@
                     ---
                    -submit: false
                    +submit: true
                     ---
                    -Original comment.
                    +Approved fixture document.
                "}
                .to_owned(),
            ),
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
        let request =
            Request::post("/sessions/fixture-session/review-documents/fixture-review/finish")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "id": "fixture-review",
                        "revision": current_revision(&document_path),
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
            let bytes_read = read_fifo_signal_or_pending(&mut finish_reader, &mut finish_signal)
                .expect("finish request FIFO read should succeed");
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
            crate::shared::human_in_the_loop::signal_fifo_with_byte(&finish_ack_path, b'0');
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
                serde_json::json!({
                    "success": true,
                    "revision": current_revision(&document_path),
                }),
                "Saved before review closes.".to_string(),
                "Saved before review closes.".to_string(),
                1,
                *b"0",
                0,
                [0],
            ),
        );
    }

    #[cfg(unix)]
    #[rstest]
    #[tokio::test]
    async fn finish_returns_gateway_timeout_when_review_process_does_not_acknowledge(
        active_review_document: ActiveReviewDocument,
    ) {
        use std::io::Read;
        use std::os::unix::fs::OpenOptionsExt;

        let document_path = active_review_document.document_path;
        let mut finish_reader = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(finish_request_fifo_path(&document_path))
            .expect("finish request FIFO reader should be opened");
        let session = active_review_document.session;
        let app = router_with_session_loader(move |_| Ok(Some(session.clone())));
        let response = app
            .oneshot(
                Request::post("/sessions/fixture-session/review-document/finish")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "id": "fixture-review",
                            "revision": current_revision(&document_path),
                            "frontmatter": null,
                            "body": "Saved without a finish acknowledgement.",
                        })
                        .to_string(),
                    ))
                    .expect("request should be valid"),
            )
            .await
            .expect("request should complete");
        let mut finish_signal = [0_u8; 1];
        let finish_bytes_read = finish_reader
            .read(&mut finish_signal)
            .expect("finish request FIFO should be readable");

        assert_eq!(
            (
                response.status(),
                finish_bytes_read,
                finish_signal,
                std::fs::read_to_string(&document_path).expect("saved document should be readable"),
            ),
            (
                StatusCode::GATEWAY_TIMEOUT,
                1,
                *b"0",
                "Saved without a finish acknowledgement.".to_string(),
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
                            "revision": current_revision(&document_path),
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
