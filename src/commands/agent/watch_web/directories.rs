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
        let config = crate::shared::config::load_config_or_default();
        ghq_root_from_config(&config)
    }

    fn home_dir(&self) -> Option<PathBuf> {
        crate::shared::dirs::home_dir()
    }
}

fn ghq_root_from_config(config: &crate::shared::config::Config) -> Option<PathBuf> {
    crate::shared::repos_root::resolve_repos_root(config.agent.worktree.repos_root.as_deref()).ok()
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
    let repos_root = source.ghq_root().and_then(|root| root.canonicalize().ok());
    let repositories = repos_root
        .as_deref()
        .map(discover_repositories)
        .unwrap_or_default();
    let mut sessions = sessions;
    sessions.sort_by_key(|session| std::cmp::Reverse(session.updated_at));

    let mut directories = Vec::new();
    let mut seen = HashSet::new();
    for session in sessions {
        let Ok(path) = session.cwd.canonicalize() else {
            continue;
        };
        if !path.is_dir() {
            continue;
        }
        let Some(repo) = repository_for_session(&path, &repositories) else {
            continue;
        };
        if matches_query(
            repository_relative_path(repo, repos_root.as_deref()),
            needle.as_deref(),
        ) {
            push_directory(
                &mut directories,
                &mut seen,
                repo.clone(),
                DirectorySourceKind::Recent,
            );
        }
    }

    if let Some(needle) = needle.as_deref() {
        for repo in &repositories {
            if matches_query(
                repository_relative_path(repo, repos_root.as_deref()),
                Some(needle),
            ) {
                push_directory(
                    &mut directories,
                    &mut seen,
                    repo.clone(),
                    DirectorySourceKind::Repo,
                );
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

fn repository_for_session<'a>(path: &Path, repositories: &'a [PathBuf]) -> Option<&'a PathBuf> {
    repositories
        .iter()
        .filter(|repo| path.starts_with(repo))
        .max_by_key(|repo| repo.components().count())
}

fn repository_relative_path<'a>(repo: &'a Path, repos_root: Option<&Path>) -> &'a Path {
    repos_root
        .and_then(|root| repo.strip_prefix(root).ok())
        .unwrap_or(repo)
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
    repositories = repositories
        .into_iter()
        .filter_map(|repo| repo.canonicalize().ok())
        .filter(|repo| repo.starts_with(root))
        .collect();
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
    use rstest::rstest;
    use serde_json::Value;
    use tower::ServiceExt;

    #[test]
    fn configured_repository_root_is_used_for_search() {
        let root = tempfile::tempdir().unwrap();
        let mut config = crate::shared::config::Config::default();
        config.agent.worktree.repos_root = Some(root.path().to_string_lossy().into_owned());

        assert_eq!(
            ghq_root_from_config(&config),
            Some(root.path().to_path_buf())
        );
    }

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
            created_pr_urls: Vec::new(),
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
    async fn returns_recent_repositories_newest_first_without_worktrees_or_outside_paths() {
        let repos_root = tempfile::tempdir().unwrap();
        let older_repo = repos_root
            .path()
            .join("github.com/example.invalid/older-repo");
        let newer_repo = repos_root
            .path()
            .join("github.com/example.invalid/newer-repo");
        let older_worktree = older_repo.join(".worktrees/topic");
        let newer_worktree = newer_repo.join(".worktrees/topic");
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(older_repo.join(".git")).unwrap();
        std::fs::create_dir_all(newer_repo.join(".git")).unwrap();
        std::fs::create_dir_all(older_worktree.join("src")).unwrap();
        std::fs::create_dir_all(&newer_worktree).unwrap();
        let source = FakeDirectorySource {
            sessions: vec![
                session(older_repo.clone(), 10),
                session(newer_worktree, 30),
                session(older_worktree.join("src"), 40),
                session(outside.path().to_path_buf(), 50),
            ],
            ghq_root: Some(repos_root.path().to_path_buf()),
            home_dir: None,
        };
        let router = router_with_source(Arc::new(source));

        assert_eq!(
            get(&router, "/directories").await,
            (
                StatusCode::OK,
                serde_json::json!({
                    "directories": [
                        {"path": older_repo.canonicalize().unwrap().to_string_lossy(), "source": "recent"},
                        {"path": newer_repo.canonicalize().unwrap().to_string_lossy(), "source": "recent"},
                    ],
                }),
            ),
        );
    }

    #[tokio::test]
    async fn omits_session_directories_that_are_no_longer_available() {
        let repos_root = tempfile::tempdir().unwrap();
        let repo = repos_root
            .path()
            .join("github.com/example.invalid/active-repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let stale = repo.join(".worktrees/removed-directory");
        let source = FakeDirectorySource {
            sessions: vec![session(repo.clone(), 20), session(stale, 30)],
            ghq_root: Some(repos_root.path().to_path_buf()),
            home_dir: None,
        };
        let router = router_with_source(Arc::new(source));

        assert_eq!(
            get(&router, "/directories").await,
            (
                StatusCode::OK,
                serde_json::json!({
                    "directories": [
                        {"path": repo.canonicalize().unwrap().to_string_lossy(), "source": "recent"},
                    ],
                }),
            ),
        );
    }

    #[tokio::test]
    async fn search_includes_matching_ghq_repositories() {
        let home = tempfile::tempdir().unwrap();
        let ghq = tempfile::tempdir().unwrap();
        let repo = ghq.path().join("github.com/example.invalid/sample-app");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let source = FakeDirectorySource {
            sessions: Vec::new(),
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
    async fn search_matches_recent_repository_by_relative_path() {
        let repos_root = tempfile::tempdir().unwrap();
        let repo = repos_root
            .path()
            .join("github.com/example.invalid/sample-app");
        let worktree = repo.join(".worktrees/topic");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(&worktree).unwrap();
        let source = FakeDirectorySource {
            sessions: vec![session(worktree, 10)],
            ghq_root: Some(repos_root.path().to_path_buf()),
            home_dir: None,
        };
        let router = router_with_source(Arc::new(source));

        assert_eq!(
            get(
                &router,
                "/directories?q=github.com/example.invalid/sample-app"
            )
            .await,
            (
                StatusCode::OK,
                serde_json::json!({
                    "directories": [{
                        "path": repo.canonicalize().unwrap().to_string_lossy(),
                        "source": "recent",
                    }],
                }),
            ),
        );
    }

    #[rstest]
    #[case::tilde("~/sample-directory")]
    #[case::absolute_path("")]
    #[tokio::test]
    async fn search_includes_an_existing_direct_path(#[case] query: &str) {
        let home = tempfile::tempdir().unwrap();
        let path_candidate = home.path().join("sample-directory");
        std::fs::create_dir_all(&path_candidate).unwrap();
        let source = FakeDirectorySource {
            sessions: Vec::new(),
            ghq_root: None,
            home_dir: Some(home.path().to_path_buf()),
        };
        let router = router_with_source(Arc::new(source));
        let query = if query.is_empty() {
            path_candidate.to_string_lossy().into_owned()
        } else {
            query.to_owned()
        };

        assert_eq!(
            get(&router, &format!("/directories?q={query}")).await,
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
