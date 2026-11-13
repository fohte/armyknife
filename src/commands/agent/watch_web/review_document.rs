use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Path as RoutePath, State};
use axum::http::StatusCode;
use axum::routing::get;
use serde::Serialize;

use crate::commands::agent::review_queue::{ReviewDocumentQueue, review_document_queue};
use crate::commands::agent::store;
use crate::commands::agent::types::Session;
use crate::shared::human_in_the_loop::{LockGuard, ReviewDocumentMarker, review_document_marker};

type SessionLoader = Arc<dyn Fn(&str) -> anyhow::Result<Option<Session>> + Send + Sync>;
type ReviewQueueLoader = Arc<dyn Fn(&Session) -> ReviewDocumentQueue + Send + Sync>;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewDocumentResponse {
    id: String,
    frontmatter: serde_json::Value,
    body: String,
}

pub(super) fn router() -> Router {
    router_with_loaders(store::load_session, review_document_queue)
}

#[cfg(test)]
fn router_with_session_loader(
    load_session: impl Fn(&str) -> anyhow::Result<Option<Session>> + Send + Sync + 'static,
) -> Router {
    router_with_loaders(load_session, |_| ReviewDocumentQueue::Unmanaged)
}

fn router_with_loaders(
    load_session: impl Fn(&str) -> anyhow::Result<Option<Session>> + Send + Sync + 'static,
    load_review_queue: impl Fn(&Session) -> ReviewDocumentQueue + Send + Sync + 'static,
) -> Router {
    Router::new()
        .route("/sessions/{id}/review-document", get(get_review_document))
        .with_state((
            Arc::new(load_session) as SessionLoader,
            Arc::new(load_review_queue) as ReviewQueueLoader,
        ))
}

async fn get_review_document(
    State((load_session, load_review_queue)): State<(SessionLoader, ReviewQueueLoader)>,
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

    use super::{router_with_loaders, router_with_session_loader};

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
}
