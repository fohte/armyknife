use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::commands::agent::store;
use crate::commands::agent::types::Session;
use crate::commands::agent::watch_web::paths::normalize_directory;

pub(super) fn router() -> Router {
    router_with_source(Arc::new(LiveDirectorySource))
}

fn router_with_source(source: Arc<dyn DirectorySource>) -> Router {
    Router::new()
        .route("/directories", get(directories))
        .with_state(DirectoryState { source })
}

#[derive(Clone)]
struct DirectoryState {
    source: Arc<dyn DirectorySource>,
}

trait DirectorySource: Send + Sync {
    fn sessions(&self) -> anyhow::Result<Vec<Session>>;
    fn ghq_root(&self) -> Option<PathBuf>;
    fn home_dir(&self) -> Option<PathBuf>;
}

struct LiveDirectorySource;

impl DirectorySource for LiveDirectorySource {
    fn sessions(&self) -> anyhow::Result<Vec<Session>> {
        store::list_all_sessions()
    }

    fn ghq_root(&self) -> Option<PathBuf> {
        crate::shared::repos_root::resolve_repos_root(None).ok()
    }

    fn home_dir(&self) -> Option<PathBuf> {
        crate::shared::dirs::home_dir()
    }
}

#[derive(Debug, Deserialize)]
struct DirectoryQuery {
    q: Option<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum DirectorySourceKind {
    Recent,
    Repo,
    Path,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct DirectoryView {
    path: String,
    source: DirectorySourceKind,
}

#[derive(Serialize)]
struct DirectoriesResponse {
    directories: Vec<DirectoryView>,
}

#[derive(Serialize)]
struct DirectoryError {
    error: String,
}

async fn directories(
    State(state): State<DirectoryState>,
    Query(query): Query<DirectoryQuery>,
) -> Result<Json<DirectoriesResponse>, (StatusCode, Json<DirectoryError>)> {
    let source = state.source;
    let response = tokio::task::spawn_blocking(move || build_directory_list(source, query.q))
        .await
        .map_err(|error| internal_error(error.to_string()))?
        .map_err(|error| internal_error(error.to_string()))?;
    Ok(Json(response))
}

fn build_directory_list(
    source: Arc<dyn DirectorySource>,
    query: Option<String>,
) -> anyhow::Result<DirectoriesResponse> {
    let sessions = source.sessions()?;
    let query = query.filter(|query| !query.trim().is_empty());
    let needle = query.as_deref().map(str::to_lowercase);
    let mut sessions = sessions;
    sessions.sort_by_key(|session| std::cmp::Reverse(session.updated_at));

    let mut directories = Vec::new();
    let mut seen = HashSet::new();
    for session in sessions {
        let path = session.cwd;
        if matches_query(&path, needle.as_deref()) {
            push_directory(
                &mut directories,
                &mut seen,
                path,
                DirectorySourceKind::Recent,
            );
        }
    }

    if let (Some(needle), Some(root)) = (needle.as_deref(), source.ghq_root()) {
        for repo in discover_repositories(&root) {
            if matches_query(&repo, Some(needle)) {
                push_directory(&mut directories, &mut seen, repo, DirectorySourceKind::Repo);
            }
        }
    }

    if let Some(query) = query.as_deref()
        && let Some(path) = search_path(query, source.home_dir())
    {
        push_directory(&mut directories, &mut seen, path, DirectorySourceKind::Path);
    }

    Ok(DirectoriesResponse { directories })
}

fn matches_query(path: &Path, query: Option<&str>) -> bool {
    query.is_none_or(|query| path.to_string_lossy().to_lowercase().contains(query))
}

fn push_directory(
    output: &mut Vec<DirectoryView>,
    seen: &mut HashSet<PathBuf>,
    path: PathBuf,
    source: DirectorySourceKind,
) {
    let Ok(normalized) = path.canonicalize() else {
        return;
    };
    if !normalized.is_dir() {
        return;
    }
    if seen.insert(normalized.clone()) {
        output.push(DirectoryView {
            path: normalized.to_string_lossy().into_owned(),
            source,
        });
    }
}

fn search_path(query: &str, home: Option<PathBuf>) -> Option<PathBuf> {
    if !(query == "~" || query.starts_with("~/") || Path::new(query).is_absolute()) {
        return None;
    }
    normalize_directory(query, home.as_deref()).ok()
}

fn discover_repositories(root: &Path) -> Vec<PathBuf> {
    let mut repositories = Vec::new();
    collect_repositories(root, 0, &mut repositories);
    repositories.sort();
    repositories
}

fn collect_repositories(directory: &Path, depth: usize, output: &mut Vec<PathBuf>) {
    if depth == 3 {
        if directory.join(".git").exists() {
            output.push(directory.to_path_buf());
        }
        return;
    }

    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir()
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| !name.starts_with('.'))
        {
            collect_repositories(&path, depth + 1, output);
        }
    }
}

fn internal_error(error: String) -> (StatusCode, Json<DirectoryError>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(DirectoryError { error }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use chrono::{TimeZone, Utc};
    use serde_json::Value;
    use tower::ServiceExt;

    #[derive(Clone)]
    struct FakeDirectorySource {
        sessions: Vec<Session>,
        ghq_root: Option<PathBuf>,
        home_dir: Option<PathBuf>,
    }

    impl DirectorySource for FakeDirectorySource {
        fn sessions(&self) -> anyhow::Result<Vec<Session>> {
            Ok(self.sessions.clone())
        }

        fn ghq_root(&self) -> Option<PathBuf> {
            self.ghq_root.clone()
        }

        fn home_dir(&self) -> Option<PathBuf> {
            self.home_dir.clone()
        }
    }

    fn session(cwd: PathBuf, updated_at: i64) -> Session {
        let timestamp = Utc.timestamp_opt(updated_at, 0).unwrap();
        Session {
            launch_id: None,
            session_id: format!("session-{updated_at}"),
            archived_at: None,
            pr_context: None,
            crit_urls: Vec::new(),
            web_uis: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd,
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: crate::commands::agent::types::SessionStatus::Stopped,
            agent_status: None,
            agent_status_clear_pending: false,
            created_at: timestamp,
            updated_at: timestamp,
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
            pending_bg_task_ids: Default::default(),
            pending_agent_task_ids: Default::default(),
            pending_permission_agent_ids: Default::default(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            engine: crate::commands::agent::types::Engine::Claude,
            model: None,
            effort: None,
        }
    }

    async fn get(router: &Router, uri: &str) -> (StatusCode, Value) {
        let response = router
            .clone()
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn returns_recent_session_directories_newest_first_without_duplicates() {
        let older = tempfile::tempdir().unwrap();
        let newer = tempfile::tempdir().unwrap();
        let source = FakeDirectorySource {
            sessions: vec![
                session(older.path().to_path_buf(), 10),
                session(newer.path().to_path_buf(), 30),
                session(older.path().to_path_buf(), 20),
            ],
            ghq_root: None,
            home_dir: None,
        };
        let router = router_with_source(Arc::new(source));

        assert_eq!(
            get(&router, "/directories").await,
            (
                StatusCode::OK,
                serde_json::json!({
                    "directories": [
                        {"path": newer.path().canonicalize().unwrap().to_string_lossy(), "source": "recent"},
                        {"path": older.path().canonicalize().unwrap().to_string_lossy(), "source": "recent"},
                    ],
                }),
            ),
        );
    }

    #[tokio::test]
    async fn omits_session_directories_that_are_no_longer_available() {
        let existing = tempfile::tempdir().unwrap();
        let stale = existing.path().join("removed-directory");
        let source = FakeDirectorySource {
            sessions: vec![
                session(existing.path().to_path_buf(), 20),
                session(stale, 30),
            ],
            ghq_root: None,
            home_dir: None,
        };
        let router = router_with_source(Arc::new(source));

        assert_eq!(
            get(&router, "/directories").await,
            (
                StatusCode::OK,
                serde_json::json!({
                    "directories": [
                        {"path": existing.path().canonicalize().unwrap().to_string_lossy(), "source": "recent"},
                    ],
                }),
            ),
        );
    }

    #[tokio::test]
    async fn search_includes_matching_ghq_repositories() {
        let home = tempfile::tempdir().unwrap();
        let ghq = tempfile::tempdir().unwrap();
        let recent = home.path().join("recent-project");
        let repo = ghq.path().join("github.com/example/sample-app");
        std::fs::create_dir_all(&recent).unwrap();
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let source = FakeDirectorySource {
            sessions: vec![session(recent, 10)],
            ghq_root: Some(ghq.path().to_path_buf()),
            home_dir: Some(home.path().to_path_buf()),
        };
        let router = router_with_source(Arc::new(source));

        assert_eq!(
            get(&router, "/directories?q=sample").await,
            (
                StatusCode::OK,
                serde_json::json!({
                    "directories": [
                        {"path": repo.canonicalize().unwrap().to_string_lossy(), "source": "repo"},
                    ],
                }),
            ),
        );
    }

    #[tokio::test]
    async fn search_includes_an_existing_tilde_path() {
        let home = tempfile::tempdir().unwrap();
        let path_candidate = home.path().join("sample-directory");
        std::fs::create_dir_all(&path_candidate).unwrap();
        let source = FakeDirectorySource {
            sessions: Vec::new(),
            ghq_root: None,
            home_dir: Some(home.path().to_path_buf()),
        };
        let router = router_with_source(Arc::new(source));

        assert_eq!(
            get(&router, "/directories?q=~/sample-directory").await,
            (
                StatusCode::OK,
                serde_json::json!({
                    "directories": [{
                        "path": path_candidate.canonicalize().unwrap().to_string_lossy(),
                        "source": "path",
                    }],
                }),
            ),
        );
    }
}
