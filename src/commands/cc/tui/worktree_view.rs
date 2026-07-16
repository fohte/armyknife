//! Worktree view state and data for the cc watch TUI.
//!
//! The worktree view enumerates all linked worktrees across discovered
//! repositories under ghq, groups them by repository, and overlays the
//! session count + active-session marker so the user can spot worktrees
//! whose sessions all died.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;
use ratatui::widgets::ListState;

use super::worktree_session_children::{SessionChild, sessions_under_worktree_from_canonical};
use crate::commands::cc::types::Session;
use crate::shared::active_session::{NoActivityProbe, is_session_active};

/// One discovered linked worktree, combined with how many cc sessions
/// currently live inside it.
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
    /// Number of cc sessions whose cwd is inside this worktree.
    pub session_count: usize,
    /// True if at least one session inside is "active" per shared probe.
    pub has_active: bool,
    /// Sessions living under this worktree, newest-first. Populated by
    /// `refresh_session_overlay`.
    pub sessions: Vec<SessionChild>,
}

/// Symbol used in the status column for this worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeStatus {
    /// No sessions in this worktree (orphan candidate).
    Orphan,
    /// Has an active session.
    Active,
    /// Has sessions but none are active.
    Idle,
}

impl WorktreeRow {
    pub fn status(&self) -> WorktreeStatus {
        if self.session_count == 0 {
            WorktreeStatus::Orphan
        } else if self.has_active {
            WorktreeStatus::Active
        } else {
            WorktreeStatus::Idle
        }
    }
}

/// Sub-mode within the worktree view.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum WorktreeMode {
    #[default]
    Normal,
    /// Pending `d` confirmation for a worktree.
    Confirm {
        worktree_path: PathBuf,
        session_count: usize,
        /// True when at least one session inside is currently active.
        /// Used to raise a louder warning in the confirm prompt.
        has_active: bool,
    },
}

/// An entry in the rendered worktree list — a repo group header, a
/// selectable worktree row, or a selectable session row nested under
/// a worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeListEntry {
    RepoHeader(String),
    Worktree(WorktreeRow),
    Session(SessionChild),
}

/// Loading state of the background worktree discovery.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum WorktreeLoadState {
    #[default]
    Loading,
    Loaded(Vec<WorktreeRow>),
    Failed(String),
}

/// Persistent state for the worktree view.
#[derive(Debug, Default)]
pub struct WorktreeView {
    pub state: WorktreeLoadState,
    pub mode: WorktreeMode,
    pub list_state: ListState,
}

impl WorktreeView {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the loaded set; also overlays current session counts.
    pub fn set_rows(&mut self, mut rows: Vec<WorktreeRow>) {
        rows.sort_by(|a, b| {
            a.repo
                .cmp(&b.repo)
                .then_with(|| a.branch.cmp(&b.branch))
                .then_with(|| a.name.cmp(&b.name))
        });
        self.state = WorktreeLoadState::Loaded(rows);
        self.select_first_worktree();
    }

    pub fn set_failed(&mut self, error: String) {
        self.state = WorktreeLoadState::Failed(error);
    }

    /// Recomputes session_count / has_active for the currently loaded rows
    /// based on the latest session list. Called whenever sessions change.
    pub fn refresh_session_overlay(&mut self, sessions: &[Session]) {
        let WorktreeLoadState::Loaded(rows) = &mut self.state else {
            return;
        };
        let now = Utc::now();
        let timeout = Duration::from_secs(60);
        let probe = NoActivityProbe;

        // Canonicalize once per session so an N-rows × M-sessions refresh
        // does not hit the filesystem N×M times.
        let canonical_sessions: Vec<(PathBuf, &Session)> = sessions
            .iter()
            .map(|s| (canonicalize_or_self(&s.cwd), s))
            .collect();

        for row in rows.iter_mut() {
            // `row.path` is already canonicalized at discovery time.
            let in_wt: Vec<&Session> = canonical_sessions
                .iter()
                .filter(|(c, _)| c.starts_with(&row.path))
                .map(|(_, s)| *s)
                .collect();
            row.session_count = in_wt.len();
            row.has_active = in_wt
                .iter()
                .any(|s| is_session_active(s, &probe, now, timeout));
            row.sessions = sessions_under_worktree_from_canonical(&row.path, &canonical_sessions);
        }
    }

    /// Loaded rows with grouped headers, ready for rendering / selection.
    pub fn list_entries(&self) -> Vec<WorktreeListEntry> {
        let WorktreeLoadState::Loaded(rows) = &self.state else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut current_repo: Option<&str> = None;
        for row in rows {
            if current_repo != Some(row.repo.as_str()) {
                out.push(WorktreeListEntry::RepoHeader(row.repo.clone()));
                current_repo = Some(row.repo.as_str());
            }
            out.push(WorktreeListEntry::Worktree(row.clone()));
            for s in &row.sessions {
                out.push(WorktreeListEntry::Session(s.clone()));
            }
        }
        out
    }

    /// Indices in `list_entries()` that point to selectable rows
    /// (worktree rows + nested session rows). Repo headers are skipped.
    pub fn selectable_indices(&self) -> Vec<usize> {
        self.list_entries()
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                matches!(
                    e,
                    WorktreeListEntry::Worktree(_) | WorktreeListEntry::Session(_)
                )
                .then_some(i)
            })
            .collect()
    }

    /// Indices that point to worktree rows specifically — used for
    /// number-jump (`1`..`9`) where session rows are not navigable.
    fn worktree_indices(&self) -> Vec<usize> {
        self.list_entries()
            .iter()
            .enumerate()
            .filter_map(|(i, e)| matches!(e, WorktreeListEntry::Worktree(_)).then_some(i))
            .collect()
    }

    pub fn select_first_worktree(&mut self) {
        // Prefer the first worktree row over any session row so the
        // cursor lands somewhere meaningful for `d` (delete) without
        // requiring the user to scroll past nested sessions.
        if let Some(&i) = self.worktree_indices().first() {
            self.list_state.select(Some(i));
        } else if let Some(&i) = self.selectable_indices().first() {
            self.list_state.select(Some(i));
        } else {
            self.list_state.select(None);
        }
    }

    /// Steps the selection by `delta` (positive = down, negative = up),
    /// wrapping around at both ends. Headers are always skipped.
    fn step(&mut self, delta: isize) {
        let sel = self.selectable_indices();
        if sel.is_empty() {
            return;
        }
        // Look up the *exact* selected index in `sel`. If the previous
        // selection no longer lives in the selectable set (rows changed,
        // user landed mid-header etc.), restart at the first selectable.
        let cur_pos = self
            .list_state
            .selected()
            .and_then(|c| sel.iter().position(|&i| i == c));
        let len = sel.len() as isize;
        let next = match cur_pos {
            Some(p) => (((p as isize) + delta).rem_euclid(len)) as usize,
            None => 0,
        };
        self.list_state.select(Some(sel[next]));
    }

    pub fn select_next(&mut self) {
        self.step(1);
    }

    pub fn select_previous(&mut self) {
        self.step(-1);
    }

    pub fn select_by_number(&mut self, num: usize) {
        let wt = self.worktree_indices();
        if num > 0 && num <= wt.len() {
            self.list_state.select(Some(wt[num - 1]));
        }
    }

    pub fn selected_worktree(&self) -> Option<WorktreeRow> {
        let idx = self.list_state.selected()?;
        let entries = self.list_entries();
        match entries.get(idx)? {
            WorktreeListEntry::Worktree(row) => Some(row.clone()),
            _ => None,
        }
    }

    /// Returns the session row currently under the cursor, if any.
    pub fn selected_session_child(&self) -> Option<SessionChild> {
        let idx = self.list_state.selected()?;
        match self.list_entries().get(idx)? {
            WorktreeListEntry::Session(s) => Some(s.clone()),
            _ => None,
        }
    }
}

/// `Path::canonicalize`, falling back to the original path on error.
/// macOS `/tmp` and `/var` are symlinks to `/private/...`, so callers that
/// want to match cwds against worktree paths must compare the realpath form
/// on both sides.
pub fn canonicalize_or_self(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// Canonicalizes both sides: macOS `/tmp` → `/private/tmp` etc. would
/// otherwise make a plain `starts_with` falsely return false.
pub fn session_lives_under(session_cwd: &Path, worktree_path: &Path) -> bool {
    canonicalize_or_self(session_cwd).starts_with(canonicalize_or_self(worktree_path))
}

/// Discover all linked worktrees under `repos_root` and return them as
/// `WorktreeRow` entries without session-count overlay. Pure I/O — meant
/// to run on a background thread.
pub fn discover_worktree_rows(repos_root: &Path, worktrees_dir: &str) -> Vec<WorktreeRow> {
    use crate::commands::wm::worktree::list_linked_worktrees;
    use crate::infra::git::open_repo_at;
    use crate::shared::repos_root::discover_repos_with_worktrees;

    let mut out = Vec::new();
    for repo_path in discover_repos_with_worktrees(repos_root, worktrees_dir) {
        let Ok(repo) = open_repo_at(&repo_path) else {
            continue;
        };
        let repo_name = repo_path
            .file_name()
            .and_then(|n| n.to_str())
            .map(String::from)
            .unwrap_or_else(|| repo_path.display().to_string());
        let Ok(linked) = list_linked_worktrees(&repo) else {
            continue;
        };
        for wt in linked {
            let name = wt
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .map(String::from)
                .unwrap_or_else(|| wt.path.display().to_string());
            out.push(WorktreeRow {
                repo: repo_name.clone(),
                branch: wt.branch,
                name,
                // Canonicalize once on the discovery thread; downstream
                // overlay refreshes happen on the UI thread and would
                // otherwise hit the filesystem per row, per refresh.
                path: canonicalize_or_self(&wt.path),
                session_count: 0,
                has_active: false,
                sessions: Vec::new(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::types::SessionStatus;
    use chrono::{DateTime, Utc};
    use rstest::{fixture, rstest};
    use std::collections::BTreeSet;

    fn row(repo: &str, branch: &str, name: &str, path: &str) -> WorktreeRow {
        WorktreeRow {
            repo: repo.to_string(),
            branch: branch.to_string(),
            name: name.to_string(),
            path: PathBuf::from(path),
            session_count: 0,
            has_active: false,
            sessions: Vec::new(),
        }
    }

    fn session_at(id: &str, cwd: PathBuf, status: SessionStatus) -> Session {
        Session {
            session_id: id.to_string(),
            cwd,
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status,
            created_at: now(),
            updated_at: now(),
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: BTreeSet::new(),
            read_at: None,
            sweep_signaled: false,
        }
    }

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    #[fixture]
    fn view_with_rows() -> WorktreeView {
        let mut v = WorktreeView::new();
        v.set_rows(vec![
            row("repo1", "feat/a", "feat-a", "/tmp/r1/.worktrees/feat-a"),
            row("repo1", "fix/b", "fix-b", "/tmp/r1/.worktrees/fix-b"),
            row("repo2", "main2", "main2", "/tmp/r2/.worktrees/main2"),
        ]);
        v
    }

    #[rstest]
    fn list_entries_groups_by_repo(view_with_rows: WorktreeView) {
        let entries = view_with_rows.list_entries();
        assert_eq!(entries.len(), 5); // 2 headers + 3 worktrees
        assert!(matches!(&entries[0], WorktreeListEntry::RepoHeader(r) if r == "repo1"));
        assert!(matches!(&entries[1], WorktreeListEntry::Worktree(_)));
        assert!(matches!(&entries[2], WorktreeListEntry::Worktree(_)));
        assert!(matches!(&entries[3], WorktreeListEntry::RepoHeader(r) if r == "repo2"));
        assert!(matches!(&entries[4], WorktreeListEntry::Worktree(_)));
    }

    #[rstest]
    fn selectable_indices_skip_headers(view_with_rows: WorktreeView) {
        assert_eq!(view_with_rows.selectable_indices(), vec![1, 2, 4]);
    }

    #[rstest]
    fn select_next_wraps_through_worktrees(mut view_with_rows: WorktreeView) {
        // initial selection points to first worktree
        assert_eq!(view_with_rows.list_state.selected(), Some(1));
        view_with_rows.select_next();
        assert_eq!(view_with_rows.list_state.selected(), Some(2));
        view_with_rows.select_next();
        assert_eq!(view_with_rows.list_state.selected(), Some(4));
        view_with_rows.select_next();
        assert_eq!(view_with_rows.list_state.selected(), Some(1));
    }

    #[rstest]
    fn select_previous_wraps_through_worktrees(mut view_with_rows: WorktreeView) {
        assert_eq!(view_with_rows.list_state.selected(), Some(1));
        view_with_rows.select_previous();
        assert_eq!(view_with_rows.list_state.selected(), Some(4));
        view_with_rows.select_previous();
        assert_eq!(view_with_rows.list_state.selected(), Some(2));
    }

    #[rstest]
    #[case::first(1, Some(1))]
    #[case::second(2, Some(2))]
    #[case::third(3, Some(4))]
    #[case::out_of_range(9, Some(1))]
    #[case::zero(0, Some(1))]
    fn select_by_number(
        mut view_with_rows: WorktreeView,
        #[case] num: usize,
        #[case] expected: Option<usize>,
    ) {
        view_with_rows.select_by_number(num);
        assert_eq!(view_with_rows.list_state.selected(), expected);
    }

    #[rstest]
    fn selected_worktree_returns_underlying_row(view_with_rows: WorktreeView) {
        let sel = view_with_rows.selected_worktree().expect("selection");
        assert_eq!(sel.name, "feat-a");
    }

    #[rstest]
    #[case::orphan(0, false, WorktreeStatus::Orphan)]
    #[case::idle(2, false, WorktreeStatus::Idle)]
    #[case::active(2, true, WorktreeStatus::Active)]
    fn worktree_row_status(
        #[case] count: usize,
        #[case] active: bool,
        #[case] expected: WorktreeStatus,
    ) {
        let mut r = row("r", "b", "n", "/tmp/x");
        r.session_count = count;
        r.has_active = active;
        assert_eq!(r.status(), expected);
    }

    #[rstest]
    fn list_entries_nests_sessions_under_worktree() {
        let dir = tempfile::tempdir().expect("tempdir");
        let wt = dir.path().join("wt");
        std::fs::create_dir_all(&wt).expect("mkdir");

        let mut v = WorktreeView::new();
        v.set_rows(vec![WorktreeRow {
            repo: "r".to_string(),
            branch: "b".to_string(),
            name: "wt".to_string(),
            path: canonicalize_or_self(&wt),
            session_count: 0,
            has_active: false,
            sessions: Vec::new(),
        }]);
        v.refresh_session_overlay(&[
            session_at("a", wt.clone(), SessionStatus::Running),
            session_at("b", wt.clone(), SessionStatus::Running),
        ]);

        let entries = v.list_entries();
        // RepoHeader + Worktree + 2 Session
        assert_eq!(entries.len(), 4);
        assert!(matches!(entries[0], WorktreeListEntry::RepoHeader(_)));
        assert!(matches!(entries[1], WorktreeListEntry::Worktree(_)));
        assert!(matches!(entries[2], WorktreeListEntry::Session(_)));
        assert!(matches!(entries[3], WorktreeListEntry::Session(_)));

        // Both the worktree row and the two session rows are selectable.
        assert_eq!(v.selectable_indices(), vec![1, 2, 3]);
    }

    #[rstest]
    fn selected_session_child_returns_session_under_cursor() {
        let dir = tempfile::tempdir().expect("tempdir");
        let wt = dir.path().join("wt");
        std::fs::create_dir_all(&wt).expect("mkdir");

        let mut v = WorktreeView::new();
        v.set_rows(vec![WorktreeRow {
            repo: "r".to_string(),
            branch: "b".to_string(),
            name: "wt".to_string(),
            path: canonicalize_or_self(&wt),
            session_count: 0,
            has_active: false,
            sessions: Vec::new(),
        }]);
        v.refresh_session_overlay(&[session_at("only", wt.clone(), SessionStatus::Running)]);

        // Cursor on the worktree row: no session child returned.
        assert_eq!(v.list_state.selected(), Some(1));
        assert!(v.selected_session_child().is_none());

        // Step down onto the session row.
        v.select_next();
        let child = v.selected_session_child().expect("session child");
        assert_eq!(child.session_id, "only");
    }

    #[rstest]
    fn refresh_session_overlay_counts_sessions_under_path() {
        // Build a temp dir so canonicalize succeeds and starts_with checks
        // run against the same realpath the rows hold.
        let dir = tempfile::tempdir().expect("tempdir");
        let wt = dir.path().join("wt");
        std::fs::create_dir_all(&wt).expect("mkdir");
        let other = dir.path().join("other");
        std::fs::create_dir_all(&other).expect("mkdir");

        let mut v = WorktreeView::new();
        v.set_rows(vec![WorktreeRow {
            repo: "r".to_string(),
            branch: "b".to_string(),
            name: "wt".to_string(),
            // Mirror what `discover_worktree_rows` does so the overlay
            // comparison hits the same realpath on both sides.
            path: canonicalize_or_self(&wt),
            session_count: 0,
            has_active: false,
            sessions: Vec::new(),
        }]);

        let sessions = vec![
            session_at("inside", wt.clone(), SessionStatus::Running),
            session_at("outside", other.clone(), SessionStatus::Running),
        ];
        v.refresh_session_overlay(&sessions);

        let entries = v.list_entries();
        let WorktreeListEntry::Worktree(r) = &entries[1] else {
            panic!("expected worktree row");
        };
        assert_eq!(r.session_count, 1);
        assert!(r.has_active);
    }
}
