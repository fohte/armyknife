use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::super::worktree::{WorktreeDiscoveryState, WorktreeRow, canonicalize_or_self};
use super::App;
#[cfg(test)]
use crate::commands::agent::types::Engine;

impl App {
    /// Cache lookup only. Misses are expected for sessions whose async
    /// resolution has not yet completed.
    pub fn get_cached_worktree_labels(&self, cwd: &std::path::Path) -> Option<(&str, &str)> {
        self.worktree_label_cache
            .get(cwd)
            .map(|(r, n)| (r.as_str(), n.as_str()))
    }

    /// Returns cwds present in `sessions` whose worktree labels are neither
    /// cached nor currently being resolved, and marks them as pending.
    /// Callers dispatch the returned list to a background resolver.
    pub fn claim_unresolved_label_cwds(&mut self) -> Vec<PathBuf> {
        let mut seen: HashSet<&Path> = HashSet::new();
        let mut out = Vec::new();
        for session in &self.sessions {
            let cwd = session.cwd.as_path();
            if !seen.insert(cwd) {
                continue;
            }
            if self.worktree_label_cache.contains_key(cwd) {
                continue;
            }
            if self.pending_label_cwds.contains(cwd) {
                continue;
            }
            out.push(cwd.to_path_buf());
        }
        for cwd in &out {
            self.pending_label_cwds.insert(cwd.clone());
        }
        out
    }

    /// Inserts the results of an async label resolution into the cache.
    pub fn apply_resolved_labels(&mut self, results: Vec<(PathBuf, String, String)>) {
        for (cwd, repo, worktree) in results {
            self.pending_label_cwds.remove(&cwd);
            self.worktree_label_cache.insert(cwd, (repo, worktree));
        }
    }

    /// Installs the freshly discovered worktrees for Clean view.
    pub fn set_worktrees(&mut self, rows: Vec<WorktreeRow>) {
        self.worktree_discovery = WorktreeDiscoveryState::Loaded(rows);
        self.refresh_worktree_session_counts();
    }

    /// Refreshes the session counts used by Clean view without re-running
    /// git discovery.
    pub fn refresh_worktree_session_counts(&mut self) {
        let WorktreeDiscoveryState::Loaded(rows) = &mut self.worktree_discovery else {
            return;
        };
        let canonical_sessions: Vec<_> = self
            .sessions
            .iter()
            .map(|session| canonicalize_or_self(&session.cwd))
            .collect();
        for row in rows {
            row.session_count = canonical_sessions
                .iter()
                .filter(|cwd| cwd.starts_with(&row.path))
                .count();
        }
    }

    /// Marks worktree discovery as failed and surfaces the error in the global banner.
    pub fn set_worktrees_failed(&mut self, error: String) {
        self.set_error(format!("Failed to load worktrees: {error}"));
        self.worktree_discovery = WorktreeDiscoveryState::Failed(error);
        self.seed_clean_view_if_pending();
    }
}

/// Resolves session labels for the given cwds on the calling thread.
/// Intended for use by a background worker; not called from render.
pub(in crate::commands::agent::tui) fn resolve_labels_for_cwds(
    cwds: &[PathBuf],
) -> Vec<(PathBuf, String, String)> {
    cwds.iter()
        .map(|cwd| {
            let (repo, worktree) = resolve_session_labels_for_path(cwd);
            (cwd.clone(), repo, worktree)
        })
        .collect()
}

/// Resolves (repo_name, worktree_name) for `cwd` using a single libgit2
/// open. `repo_name` is the main worktree's basename; `worktree_name` is the
/// current branch when resolvable, otherwise the cwd's workdir basename.
/// Falls back to the cwd basename when the path is outside a git repo.
fn resolve_session_labels_for_path(cwd: &Path) -> (String, String) {
    use crate::infra::git::open_repo_at;

    let basename_fallback = || {
        cwd.file_name()
            .and_then(|n| n.to_str())
            .map(String::from)
            .unwrap_or_else(|| cwd.display().to_string())
    };

    let Ok(repo) = open_repo_at(cwd) else {
        let fallback = basename_fallback();
        return (fallback.clone(), fallback);
    };

    let repo_name = repo
        .main_workdir()
        .ok()
        .and_then(|p| p.file_name().and_then(|n| n.to_str()).map(String::from))
        .unwrap_or_else(basename_fallback);

    let branch = repo.current_branch().ok();
    let worktree_name = branch.filter(|b| b != "HEAD").unwrap_or_else(|| {
        let workdir = repo.workdir();
        workdir
            .file_name()
            .and_then(|n| n.to_str())
            .map(String::from)
            .unwrap_or_else(|| workdir.display().to_string())
    });

    (repo_name, worktree_name)
}

/// Resolves the git worktree root for `cwd`. Returns `None` if `cwd` is not
/// inside a repository opened as a worktree (bare main repo or non-git paths
/// are treated as "not a worktree"). The returned path is the worktree's
/// workdir, so matching sibling sessions via `starts_with` is safe even when
/// `cwd` is a subdirectory.
pub(super) fn resolve_worktree_root(cwd: &Path) -> Option<PathBuf> {
    let repo = crate::infra::git::open_repo_at(cwd).ok()?;
    if !repo.is_worktree() {
        return None;
    }
    Some(repo.workdir().to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::types::{Session, SessionStatus};
    use chrono::{TimeDelta, Utc};
    use rstest::rstest;

    /// Counter to assign distinct timestamps to test sessions.
    /// Each call returns a progressively older timestamp, so sessions
    /// created first sort first (most recent updated_at).
    use std::sync::atomic::{AtomicI64, Ordering};
    static TEST_SESSION_COUNTER: AtomicI64 = AtomicI64::new(0);

    fn create_test_session(id: &str) -> Session {
        let offset = TEST_SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
        let now = Utc::now();
        Session {
            session_id: id.to_string(),
            crit_urls: Vec::new(),
            cwd: PathBuf::from("/tmp/test"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            created_at: now - TimeDelta::seconds(offset),
            updated_at: now - TimeDelta::seconds(offset),
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: std::collections::BTreeSet::new(),
            pending_agent_task_ids: std::collections::BTreeSet::new(),
            pending_permission_agent_ids: std::collections::BTreeSet::new(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            engine: Engine::Claude,
        }
    }

    fn create_test_app(sessions: Vec<Session>) -> App {
        App::with_sessions(sessions)
    }

    // =========================================================================
    // session label resolution tests
    // =========================================================================

    #[rstest]
    #[case::normal_path("/home/user/project", "project", "project")]
    #[case::nested_path("/home/user/ghq/github.com/fohte/armyknife", "armyknife", "armyknife")]
    fn test_resolve_session_labels_fallback(
        #[case] cwd: &str,
        #[case] expected_repo: &str,
        #[case] expected_wt: &str,
    ) {
        let (repo, wt) = resolve_session_labels_for_path(&PathBuf::from(cwd));
        assert_eq!(repo, expected_repo);
        assert_eq!(wt, expected_wt);
    }

    #[test]
    fn test_get_cached_worktree_labels_miss_returns_none() {
        let app = create_test_app(vec![]);
        let cwd = PathBuf::from("/home/user/project");
        assert!(app.get_cached_worktree_labels(&cwd).is_none());
    }

    #[test]
    fn test_apply_resolved_labels_populates_cache() {
        let mut app = create_test_app(vec![]);
        let cwd = PathBuf::from("/home/user/project");

        app.apply_resolved_labels(vec![(
            cwd.clone(),
            "project".to_string(),
            "main".to_string(),
        )]);

        assert_eq!(
            app.get_cached_worktree_labels(&cwd),
            Some(("project", "main"))
        );
    }

    #[test]
    fn test_claim_unresolved_label_cwds_dedups_and_marks_pending() {
        let mut app = create_test_app(vec![create_test_session("a"), create_test_session("b")]);
        // Both default sessions share cwd `/tmp/test`, so only one cwd is returned.
        let first = app.claim_unresolved_label_cwds();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0], PathBuf::from("/tmp/test"));

        // Second call returns nothing (already pending).
        let second = app.claim_unresolved_label_cwds();
        assert!(second.is_empty());

        // After the result is applied the cwd is cached and stays cached.
        app.apply_resolved_labels(vec![(
            PathBuf::from("/tmp/test"),
            "test".to_string(),
            "main".to_string(),
        )]);
        let third = app.claim_unresolved_label_cwds();
        assert!(third.is_empty());
        assert_eq!(
            app.get_cached_worktree_labels(&PathBuf::from("/tmp/test")),
            Some(("test", "main"))
        );
    }
}
