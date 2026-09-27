use std::path::{Path, PathBuf};

/// One linked worktree discovered under the configured repositories root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeRow {
    /// Repository name (basename of repo path).
    pub repo: String,
    /// Branch name (or "(detached)" / "(unknown)") for the worktree.
    pub branch: String,
    /// Last path component of the worktree directory.
    pub name: String,
    /// Absolute path to the worktree.
    pub path: PathBuf,
    /// Number of agent sessions whose cwd is inside this worktree.
    pub session_count: usize,
}

/// Result state for the background worktree discovery used by Clean view.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum WorktreeDiscoveryState {
    #[default]
    Loading,
    Loaded(Vec<WorktreeRow>),
    Failed,
}

/// `Path::canonicalize`, falling back to the original path on error.
/// macOS `/tmp` and `/var` are symlinks to `/private/...`, so callers that
/// want to match cwds against worktree paths must compare the realpath form
/// on both sides.
pub fn canonicalize_or_self(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Discover linked worktrees under `repos_root` on a background thread.
pub fn discover_worktree_rows(repos_root: &Path, worktrees_dir: &str) -> Vec<WorktreeRow> {
    use crate::commands::wm::worktree::list_linked_worktrees;
    use crate::infra::git::open_repo_at;
    use crate::shared::repos_root::discover_repos_with_worktrees;

    let mut rows = Vec::new();
    for repo_path in discover_repos_with_worktrees(repos_root, worktrees_dir) {
        let Ok(repo) = open_repo_at(&repo_path) else {
            continue;
        };
        let repo_name = repo_path
            .file_name()
            .and_then(|name| name.to_str())
            .map(String::from)
            .unwrap_or_else(|| repo_path.display().to_string());
        let Ok(linked_worktrees) = list_linked_worktrees(&repo) else {
            continue;
        };
        for worktree in linked_worktrees {
            let name = worktree
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .map(String::from)
                .unwrap_or_else(|| worktree.path.display().to_string());
            rows.push(WorktreeRow {
                repo: repo_name.clone(),
                branch: worktree.branch,
                name,
                // Canonicalize on the discovery thread so later path matching
                // does not repeat filesystem lookups for each row.
                path: canonicalize_or_self(&worktree.path),
                session_count: 0,
            });
        }
    }
    rows
}
