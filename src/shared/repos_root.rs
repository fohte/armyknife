//! Repository root resolution and discovery for cross-repository operations.

use std::path::{Path, PathBuf};

use anyhow::Context;

/// Resolve the repos root directory.
///
/// Priority:
/// 1. Explicit config value (from config.yaml `wm.repos_root`)
/// 2. `GHQ_ROOT` environment variable (first entry if colon-separated)
/// 3. git config `ghq.root` (via gitconfig file)
/// 4. Default: `~/ghq`
pub fn resolve_repos_root(config_value: Option<&str>) -> anyhow::Result<PathBuf> {
    if let Some(path) = config_value {
        let expanded = expand_tilde(path);
        return Ok(expanded);
    }

    if let Some(path) = ghq_root_from_env() {
        return Ok(path);
    }

    if let Some(path) = ghq_root_from_gitconfig() {
        return Ok(path);
    }

    // Default: ~/ghq
    let home = super::dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join("ghq"))
}

/// Read GHQ_ROOT from environment variable.
/// Takes the first entry if colon-separated (matching ghq's filepath.SplitList behavior).
fn ghq_root_from_env() -> Option<PathBuf> {
    let val = std::env::var("GHQ_ROOT").ok().filter(|v| !v.is_empty())?;
    // GHQ_ROOT can be colon-separated; take the first entry
    let first = val.split(':').next()?;
    if first.is_empty() {
        return None;
    }
    Some(expand_tilde(first))
}

/// Read ghq.root from git global config.
fn ghq_root_from_gitconfig() -> Option<PathBuf> {
    let config = git2::Config::open_default().ok()?;
    let value = config.get_path("ghq.root").ok()?;
    Some(value)
}

/// Expand `~` prefix to the home directory.
fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = super::dirs::home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(path)
}

/// Discover git repositories under `repos_root` that have a worktrees directory.
///
/// Assumes ghq-style layout: `repos_root/host/owner/repo`.
/// Searches up to 3 levels deep and filters to repos that have the
/// specified `worktrees_dir` (e.g., ".worktrees").
pub fn discover_repos_with_worktrees(repos_root: &Path, worktrees_dir: &str) -> Vec<PathBuf> {
    let mut repos = Vec::new();
    discover_recursive(repos_root, worktrees_dir, 0, 3, &mut repos);
    repos.sort();
    repos
}

fn discover_recursive(
    dir: &Path,
    worktrees_dir: &str,
    depth: usize,
    max_depth: usize,
    repos: &mut Vec<PathBuf>,
) {
    if depth > max_depth {
        return;
    }

    // Check if this directory is a git repo with worktrees
    if dir.join(".git").exists() && dir.join(worktrees_dir).is_dir() {
        repos.push(dir.to_path_buf());
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip hidden directories (except .git which we check above)
            if let Some(name) = path.file_name().and_then(|n| n.to_str())
                && name.starts_with('.')
            {
                continue;
            }
            discover_recursive(&path, worktrees_dir, depth + 1, max_depth, repos);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use tempfile::TempDir;

    #[test]
    fn resolve_repos_root_uses_config_value() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_str().unwrap();
        let result = resolve_repos_root(Some(path)).unwrap();
        assert_eq!(result, PathBuf::from(path));
    }

    #[test]
    fn resolve_repos_root_expands_tilde_in_config() {
        temp_env::with_vars([("HOME", Some("/test/home"))], || {
            let result = resolve_repos_root(Some("~/repos")).unwrap();
            assert_eq!(result, PathBuf::from("/test/home/repos"));
        });
    }

    #[test]
    fn resolve_repos_root_uses_ghq_root_env() {
        temp_env::with_vars([("GHQ_ROOT", Some("/custom/ghq"))], || {
            let result = resolve_repos_root(None).unwrap();
            assert_eq!(result, PathBuf::from("/custom/ghq"));
        });
    }

    #[test]
    fn resolve_repos_root_takes_first_from_colon_separated_ghq_root() {
        temp_env::with_vars([("GHQ_ROOT", Some("/first/ghq:/second/ghq"))], || {
            let result = resolve_repos_root(None).unwrap();
            assert_eq!(result, PathBuf::from("/first/ghq"));
        });
    }

    #[test]
    fn resolve_repos_root_falls_back_when_no_config_or_env() {
        temp_env::with_vars(
            [("GHQ_ROOT", None::<&str>), ("HOME", Some("/test/home"))],
            || {
                let result = resolve_repos_root(None).unwrap();
                // Falls back to gitconfig ghq.root (if set) or ~/ghq.
                // Either is acceptable; the key invariant is that it doesn't error.
                assert!(
                    result == Path::new("/test/home/ghq") || ghq_root_from_gitconfig().is_some(),
                    "should fall back to ~/ghq or gitconfig ghq.root"
                );
            },
        );
    }

    #[rstest]
    fn discover_finds_repos_with_worktrees() {
        let root = TempDir::new().unwrap();

        // Create ghq-style structure: host/owner/repo
        let repo_path = root.path().join("github.com/owner/repo1");
        std::fs::create_dir_all(repo_path.join(".git")).unwrap();
        std::fs::create_dir_all(repo_path.join(".worktrees")).unwrap();

        // Repo without worktrees should be excluded
        let repo2_path = root.path().join("github.com/owner/repo2");
        std::fs::create_dir_all(repo2_path.join(".git")).unwrap();

        // Not a git repo at all
        let non_repo = root.path().join("github.com/owner/not-a-repo");
        std::fs::create_dir_all(non_repo).unwrap();

        let repos = discover_repos_with_worktrees(root.path(), ".worktrees");
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0], repo_path);
    }

    #[rstest]
    fn discover_finds_multiple_repos() {
        let root = TempDir::new().unwrap();

        let repo1 = root.path().join("github.com/owner/repo-a");
        std::fs::create_dir_all(repo1.join(".git")).unwrap();
        std::fs::create_dir_all(repo1.join(".worktrees")).unwrap();

        let repo2 = root.path().join("github.com/other/repo-b");
        std::fs::create_dir_all(repo2.join(".git")).unwrap();
        std::fs::create_dir_all(repo2.join(".worktrees")).unwrap();

        let repos = discover_repos_with_worktrees(root.path(), ".worktrees");
        assert_eq!(repos.len(), 2);
    }

    #[rstest]
    fn discover_respects_custom_worktrees_dir() {
        let root = TempDir::new().unwrap();

        let repo = root.path().join("github.com/owner/repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join(".wt")).unwrap();

        // Standard .worktrees won't match
        let repos = discover_repos_with_worktrees(root.path(), ".worktrees");
        assert_eq!(repos.len(), 0);

        // Custom dir matches
        let repos = discover_repos_with_worktrees(root.path(), ".wt");
        assert_eq!(repos.len(), 1);
    }

    #[rstest]
    fn discover_returns_empty_for_nonexistent_root() {
        let repos = discover_repos_with_worktrees(Path::new("/nonexistent/path"), ".worktrees");
        assert!(repos.is_empty());
    }

    #[rstest]
    fn discover_skips_hidden_directories() {
        let root = TempDir::new().unwrap();

        // Hidden directory containing a repo should be skipped
        let hidden_repo = root.path().join(".hidden/owner/repo");
        std::fs::create_dir_all(hidden_repo.join(".git")).unwrap();
        std::fs::create_dir_all(hidden_repo.join(".worktrees")).unwrap();

        let repos = discover_repos_with_worktrees(root.path(), ".worktrees");
        assert!(repos.is_empty());
    }
}
