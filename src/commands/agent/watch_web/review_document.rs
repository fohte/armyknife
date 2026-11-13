use std::path::Path;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Path as RoutePath, State};
use axum::http::StatusCode;
use axum::routing::get;
use serde::Serialize;

use crate::commands::agent::store;
use crate::commands::agent::types::Session;
use crate::shared::human_in_the_loop::review_document_path;

type SessionLoader = Arc<dyn Fn(&str) -> anyhow::Result<Option<Session>> + Send + Sync>;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewDocumentResponse {
    frontmatter: serde_json::Value,
    body: String,
}

pub(super) fn router() -> Router {
    router_with_session_loader(store::load_session)
}

fn router_with_session_loader(
    load_session: impl Fn(&str) -> anyhow::Result<Option<Session>> + Send + Sync + 'static,
) -> Router {
    Router::new()
        .route("/sessions/{id}/review-document", get(get_review_document))
        .with_state(Arc::new(load_session) as SessionLoader)
}

async fn get_review_document(
    State(load_session): State<SessionLoader>,
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
        let Some(path) = session
            .pending_human_review_ids
            .iter()
            .find_map(|review_id| review_document_path(review_id))
        else {
            return Ok(None);
        };
        match load_review_document(&path) {
            Ok(response) => Ok(Some(response)),
            Err(error) => {
                tracing::warn!(%error, "failed to read active review document");
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)??;

    response.map(Json).ok_or(StatusCode::NOT_FOUND)
}

fn load_review_document(path: &Path) -> anyhow::Result<ReviewDocumentResponse> {
    let content = std::fs::read_to_string(path)?;
    let (frontmatter, body) = match crate::shared::yaml_frontmatter::split_frontmatter(&content) {
        Some((_, yaml, body_offset)) => (
            serde_json::to_value(serde_yaml::from_str::<serde_yaml::Value>(yaml)?)?,
            content[body_offset..].to_owned(),
        ),
        None => (serde_json::Value::Null, content),
    };

    Ok(ReviewDocumentResponse { frontmatter, body })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use chrono::Utc;
    use indoc::indoc;
    use tempfile::TempDir;
    use tower::ServiceExt;

    use crate::commands::agent::types::{Engine, Session, SessionStatus};

    use super::router_with_session_loader;

    fn session_with_review(path: &std::path::Path) -> Session {
        Session {
            session_id: "fixture-session".to_string(),
            archived_at: None,
            pr_context: None,
            crit_urls: Vec::new(),
            pending_human_review_ids: BTreeSet::from([format!(
                "fixture-review\0{}",
                path.to_string_lossy()
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
    async fn returns_not_found_without_an_active_review_document_path() {
        let mut session = session_with_review(std::path::Path::new("/unused"));
        session.pending_human_review_ids.clear();
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
}
