use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::commands::cc::types::Session;

use super::super::worktree_view::{
    WorktreeMode, WorktreeRow, canonicalize_or_self, session_lives_under,
};
use super::{App, View};

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

    /// Cycles the active view. No-op when in `Clean`.
    pub fn cycle_view(&mut self) {
        if self.view == View::Clean {
            return;
        }
        self.view = self.view.next();
        if self.view == View::Worktree {
            // Make sure overlay reflects the latest session list whenever the
            // user lands on the worktree view.
            self.worktree_view.refresh_session_overlay(&self.sessions);
        }
    }

    /// Installs the freshly loaded worktree rows.
    pub fn set_worktrees(&mut self, rows: Vec<WorktreeRow>) {
        self.worktree_view.set_rows(rows);
        self.worktree_view.refresh_session_overlay(&self.sessions);
    }

    /// Marks worktree discovery as failed (background thread error) and
    /// also surfaces the error in the global error banner so the user
    /// notices it without switching to the worktree view first.
    pub fn set_worktrees_failed(&mut self, error: String) {
        self.set_error(format!("Failed to load worktrees: {error}"));
        self.worktree_view.set_failed(error);
    }

    /// In worktree view, returns the most recently updated session inside the
    /// currently selected worktree (used for `Enter` → focus pane).
    pub fn worktree_view_focus_session(&self) -> Option<&Session> {
        let row = self.worktree_view.selected_worktree()?;
        // `row.path` is already canonicalized at discovery time.
        self.sessions
            .iter()
            .filter(|s| canonicalize_or_self(&s.cwd).starts_with(&row.path))
            .max_by_key(|s| s.updated_at)
    }

    /// Enters Confirm sub-mode on the selected worktree (for `d`).
    pub fn worktree_view_request_delete(&mut self) {
        if let Some(row) = self.worktree_view.selected_worktree() {
            self.worktree_view.mode = WorktreeMode::Confirm {
                worktree_path: row.path,
                session_count: row.session_count,
                has_active: row.has_active,
            };
        }
    }

    /// Cancels the pending worktree-view confirmation.
    pub fn worktree_view_cancel_confirm(&mut self) {
        self.worktree_view.mode = WorktreeMode::Normal;
    }

    /// Deletes the worktree via `cleanup_worktree_resources` (git worktree,
    /// branch, tmux windows, session files). Does not consult merge status.
    pub fn worktree_view_confirm_delete(&mut self) -> anyhow::Result<()> {
        let path = match &self.worktree_view.mode {
            WorktreeMode::Confirm { worktree_path, .. } => worktree_path.clone(),
            _ => return Ok(()),
        };

        self.worktree_view.mode = WorktreeMode::Normal;

        use crate::shared::cleanup;
        let result = cleanup::cleanup_worktree_resources(&path)?;

        if result.worktree_deleted {
            // Drop sessions whose cwd is gone.
            if let Some(ref wt_root) = result.worktree_root {
                let to_remove: Vec<String> = self
                    .sessions
                    .iter()
                    .filter(|s| session_lives_under(&s.cwd, wt_root))
                    .map(|s| s.session_id.clone())
                    .collect();
                for id in &to_remove {
                    self.remove_session(id);
                }
            }
            let prev_selection = self.worktree_view.list_state.selected();
            if let super::super::worktree_view::WorktreeLoadState::Loaded(rows) =
                &mut self.worktree_view.state
            {
                rows.retain(|r| r.path != path);
            }
            self.worktree_view.refresh_session_overlay(&self.sessions);
            // Keep the cursor near the deleted row: pick the first
            // selectable index >= the old position, otherwise the last.
            let sel = self.worktree_view.selectable_indices();
            let next = prev_selection
                .and_then(|p| {
                    sel.iter()
                        .find(|&&i| i >= p)
                        .copied()
                        .or_else(|| sel.last().copied())
                })
                .or_else(|| sel.first().copied());
            self.worktree_view.list_state.select(next);
        } else {
            self.set_error(format!(
                "Worktree not deleted: {} (use `a wm clean` to investigate)",
                path.display()
            ));
        }
        Ok(())
    }
}

/// Resolves session labels for the given cwds on the calling thread.
/// Intended for use by a background worker; not called from render.
pub(in crate::commands::cc::tui) fn resolve_labels_for_cwds(
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
    use crate::commands::cc::types::{Session, SessionStatus};
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
            read_at: None,
            sweep_signaled: false,
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
