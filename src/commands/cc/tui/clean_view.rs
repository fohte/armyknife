//! Clean view state for the cc watch TUI.
//!
//! Reached by pressing `c` from session view or worktree view. Shows the
//! same worktree list as the worktree view, but partitioned into
//! "To delete" (merged PR & no active session) and "Kept" (everything
//! else). The user can toggle individual rows between sections with
//! Enter, then press `y` to spawn `a cc clean-detached` as a detached
//! child that survives the parent watch process.

use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, Utc};
use ratatui::widgets::ListState;

use super::worktree_session_children::{SessionChild, sessions_under_worktree_from_canonical};
use super::worktree_view::{WorktreeRow, canonicalize_or_self};
use crate::commands::cc::types::Session;
use crate::infra::git::MergeStatus;
use crate::infra::github::PrState;
use crate::shared::active_session::{NoActivityProbe, is_session_active};

/// Top-level state of the clean view.
///
/// Rows are populated immediately from the worktree snapshot when the
/// view opens, so `Ready` arrives before PR status. PR loading progress
/// is tracked separately on `CleanView::pr_fetch`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CleanLoadState {
    /// Rows not yet built — only seen before the initial sync build runs.
    #[default]
    LoadingPr,
    Ready(Vec<CleanRow>),
    /// Catastrophic load failure (e.g. no worktree snapshot). PR-fetch
    /// failures use [`PrFetchStatus::Failed`] and keep the row list.
    Failed(String),
}

/// Independent status of the async PR-info fetch. Drives whether the
/// section toggle is allowed and how rows render their PR column.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PrFetchStatus {
    #[default]
    Loading,
    Done,
    Failed(String),
}

/// Status label shown before the async PR fetch completes.
pub const PR_FETCHING_LABEL: &str = "fetching...";
/// Status label after the async PR fetch fails.
pub const PR_FETCH_FAILED_LABEL: &str = "PR fetch failed";

/// Which section a row currently belongs to. Defaults are computed from
/// PR status + active session presence; the user can override per row
/// with Enter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanSection {
    ToDelete,
    Kept,
}

impl CleanSection {
    pub fn toggle(self) -> Self {
        match self {
            CleanSection::ToDelete => CleanSection::Kept,
            CleanSection::Kept => CleanSection::ToDelete,
        }
    }
}

/// One worktree row in the clean view, enriched with PR status and the
/// active-session marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanRow {
    pub repo: String,
    pub branch: String,
    pub name: String,
    pub path: PathBuf,
    pub session_count: usize,
    pub has_active: bool,
    pub updated_at: Option<DateTime<Utc>>,
    /// Display-only summary of PR / activity status, e.g.
    /// `"PR #1 merged"`, `"no PR · active"`. Shown in the `[label]`
    /// slot of the row format.
    pub status_label: String,
    /// True when the latest known PR was merged. Used together with
    /// `has_active` to set the default section.
    pub pr_merged: bool,
    pub section: CleanSection,
    /// Sessions living under this worktree, newest-first.
    pub sessions: Vec<SessionChild>,
}

/// Rendered entries in display order.
///
/// `SectionHeader` and `RepoHeader` are non-selectable; `Row` and
/// `Session` are both selectable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanListEntry {
    SectionHeader { section: CleanSection, count: usize },
    RepoHeader(String),
    Row(CleanRow),
    Session(SessionChild),
}

/// Persistent state for the clean view.
///
/// `state` and `pr_fetch` are independent axes: `state` describes the
/// rendered row list (initial / ready / catastrophically failed),
/// `pr_fetch` describes the async PR-info fetch (loading / done /
/// failed). All mutators in this module are responsible for keeping
/// the two in sync — do not write only one when both should move.
#[derive(Debug, Default)]
pub struct CleanView {
    pub state: CleanLoadState,
    pub pr_fetch: PrFetchStatus,
    pub list_state: ListState,
}

impl CleanView {
    pub fn new() -> Self {
        Self::default()
    }

    /// Begin a fresh clean session: discard any prior partition and
    /// switch back to LoadingPr / PR-loading.
    pub fn reset(&mut self) {
        self.state = CleanLoadState::LoadingPr;
        self.pr_fetch = PrFetchStatus::Loading;
        self.list_state.select(None);
    }

    /// Record a PR-fetch failure without throwing away the row list.
    /// Rows still showing the "fetching..." placeholder are rewritten
    /// so the user can tell which entries never resolved.
    pub fn set_failed(&mut self, error: String) {
        if let CleanLoadState::Ready(rows) = &mut self.state {
            for row in rows.iter_mut() {
                if row.status_label == PR_FETCHING_LABEL {
                    row.status_label = PR_FETCH_FAILED_LABEL.to_string();
                }
            }
            self.pr_fetch = PrFetchStatus::Failed(error);
        } else {
            self.state = CleanLoadState::Failed(error.clone());
            self.pr_fetch = PrFetchStatus::Failed(error);
        }
    }

    /// Install the initial row list built synchronously from the
    /// worktree snapshot. PR columns are placeholders; the async fetch
    /// fills them via [`Self::apply_pr_results`].
    pub fn set_initial_rows(&mut self, mut rows: Vec<CleanRow>) {
        sort_rows(&mut rows);
        self.state = CleanLoadState::Ready(rows);
        self.pr_fetch = PrFetchStatus::Loading;
        self.select_first_row();
    }

    /// Merge PR-enriched rows into the current row list by path. Rows
    /// the async pass did not return are left as-is; new rows it
    /// returned (none expected in practice) are appended.
    pub fn apply_pr_results(&mut self, mut updated: Vec<CleanRow>) {
        // Preserve the selected row across the merge so the cursor does
        // not jump when the partition shifts (e.g. a row newly resolves
        // as "merged + no active session" and moves into ToDelete).
        let selected_path = self.selected_row().map(|r| r.path);
        let CleanLoadState::Ready(rows) = &mut self.state else {
            sort_rows(&mut updated);
            self.state = CleanLoadState::Ready(updated);
            self.pr_fetch = PrFetchStatus::Done;
            self.select_first_row();
            return;
        };
        for row in rows.iter_mut() {
            if let Some(pos) = updated.iter().position(|u| u.path == row.path) {
                *row = updated.swap_remove(pos);
            }
        }
        rows.extend(updated);
        sort_rows(rows);
        self.pr_fetch = PrFetchStatus::Done;
        if let Some(path) = selected_path {
            let new_index = self
                .list_entries()
                .iter()
                .enumerate()
                .find_map(|(i, e)| match e {
                    CleanListEntry::Row(r) if r.path == path => Some(i),
                    _ => None,
                });
            if new_index.is_some() {
                self.list_state.select(new_index);
                return;
            }
        }
        self.select_first_row();
    }

    /// Sorts deterministically so re-entering the view does not jump the
    /// cursor onto a different row when the underlying set is the same.
    #[cfg(test)]
    pub fn set_rows(&mut self, mut rows: Vec<CleanRow>) {
        sort_rows(&mut rows);
        self.state = CleanLoadState::Ready(rows);
        self.pr_fetch = PrFetchStatus::Done;
        self.select_first_row();
    }

    /// True while the section toggle should be blocked because the
    /// default partition is not yet known.
    pub fn is_pr_loading(&self) -> bool {
        matches!(self.pr_fetch, PrFetchStatus::Loading)
    }

    pub fn rows(&self) -> &[CleanRow] {
        match &self.state {
            CleanLoadState::Ready(rows) => rows,
            _ => &[],
        }
    }

    /// Build the rendered entry list with section + repo group headers.
    pub fn list_entries(&self) -> Vec<CleanListEntry> {
        let CleanLoadState::Ready(rows) = &self.state else {
            return Vec::new();
        };

        let mut out = Vec::new();
        for section in [CleanSection::ToDelete, CleanSection::Kept] {
            let section_rows: Vec<&CleanRow> =
                rows.iter().filter(|r| r.section == section).collect();
            out.push(CleanListEntry::SectionHeader {
                section,
                count: section_rows.len(),
            });
            let mut current_repo: Option<&str> = None;
            for row in section_rows {
                if current_repo != Some(row.repo.as_str()) {
                    out.push(CleanListEntry::RepoHeader(row.repo.clone()));
                    current_repo = Some(row.repo.as_str());
                }
                out.push(CleanListEntry::Row(row.clone()));
                for s in &row.sessions {
                    out.push(CleanListEntry::Session(s.clone()));
                }
            }
        }
        out
    }

    /// Selectable rows include both worktree rows and the nested
    /// session rows. Enter dispatches by variant in the key handler.
    pub fn selectable_indices(&self) -> Vec<usize> {
        self.list_entries()
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                matches!(e, CleanListEntry::Row(_) | CleanListEntry::Session(_)).then_some(i)
            })
            .collect()
    }

    /// Indices that point to worktree rows only — used by
    /// `select_first_row` so the cursor lands on a toggle target.
    fn worktree_row_indices(&self) -> Vec<usize> {
        self.list_entries()
            .iter()
            .enumerate()
            .filter_map(|(i, e)| matches!(e, CleanListEntry::Row(_)).then_some(i))
            .collect()
    }

    pub fn select_first_row(&mut self) {
        // Prefer landing on a worktree row over a nested session so
        // Enter immediately maps to a toggle action.
        if let Some(&i) = self.worktree_row_indices().first() {
            self.list_state.select(Some(i));
        } else if let Some(&i) = self.selectable_indices().first() {
            self.list_state.select(Some(i));
        } else {
            self.list_state.select(None);
        }
    }

    fn step(&mut self, delta: isize) {
        let sel = self.selectable_indices();
        if sel.is_empty() {
            return;
        }
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

    /// Returns the row currently under the cursor, if any.
    pub fn selected_row(&self) -> Option<CleanRow> {
        let idx = self.list_state.selected()?;
        match self.list_entries().get(idx)? {
            CleanListEntry::Row(r) => Some(r.clone()),
            _ => None,
        }
    }

    /// Returns the session child currently under the cursor, if any.
    pub fn selected_session_child(&self) -> Option<SessionChild> {
        let idx = self.list_state.selected()?;
        match self.list_entries().get(idx)? {
            CleanListEntry::Session(s) => Some(s.clone()),
            _ => None,
        }
    }

    /// Toggle the section of the row at the current cursor. Returns true
    /// if a toggle actually happened. The cursor is re-anchored to the
    /// same row in its new section so the user sees their action take
    /// effect without losing focus.
    pub fn toggle_selected_section(&mut self) -> bool {
        // Toggle requires the resolved partition; block until PR fetch
        // completes.
        if self.is_pr_loading() {
            return false;
        }
        let Some(current) = self.selected_row() else {
            return false;
        };
        let CleanLoadState::Ready(rows) = &mut self.state else {
            return false;
        };
        let Some(row) = rows.iter_mut().find(|r| r.path == current.path) else {
            return false;
        };
        row.section = row.section.toggle();
        let target_path = row.path.clone();

        sort_rows(rows);

        // Re-anchor the cursor on the same worktree in its new section.
        let new_index = self
            .list_entries()
            .iter()
            .enumerate()
            .find_map(|(i, e)| match e {
                CleanListEntry::Row(r) if r.path == target_path => Some(i),
                _ => None,
            });
        self.list_state.select(new_index);
        true
    }

    /// All paths currently in the To delete section, in display order.
    pub fn to_delete_paths(&self) -> Vec<PathBuf> {
        self.rows()
            .iter()
            .filter(|r| r.section == CleanSection::ToDelete)
            .map(|r| r.path.clone())
            .collect()
    }

    /// Counts of (to_delete, kept_with_active).
    pub fn summary(&self) -> (usize, usize) {
        let to_delete = self
            .rows()
            .iter()
            .filter(|r| r.section == CleanSection::ToDelete)
            .count();
        let kept_active = self
            .rows()
            .iter()
            .filter(|r| r.section == CleanSection::Kept && r.has_active)
            .count();
        (to_delete, kept_active)
    }

    /// Remove rows whose path matches any of `paths`. Used by the
    /// progress watcher to drop entries the detached child confirms
    /// it deleted, without re-running discovery.
    pub fn remove_paths(&mut self, paths: &[PathBuf]) {
        let CleanLoadState::Ready(rows) = &mut self.state else {
            return;
        };
        rows.retain(|r| !paths.iter().any(|p| p == &r.path));
    }
}

fn section_order(section: CleanSection) -> u8 {
    match section {
        CleanSection::ToDelete => 0,
        CleanSection::Kept => 1,
    }
}

fn sort_rows(rows: &mut [CleanRow]) {
    rows.sort_by(|a, b| {
        section_order(a.section)
            .cmp(&section_order(b.section))
            .then_with(|| a.repo.cmp(&b.repo))
            .then_with(|| a.branch.cmp(&b.branch))
            .then_with(|| a.name.cmp(&b.name))
    });
}

/// Input pair to [`build_clean_rows`]: the worktree the user picked plus
/// the merge status fetched from GitHub. `merge_status` is `None` when
/// no PR was found or the lookup failed; rows without PR info default
/// to Kept.
#[derive(Debug, Clone)]
pub struct CleanRowInput {
    pub row: WorktreeRow,
    pub merge_status: Option<MergeStatus>,
    pub pr_number: Option<u64>,
    pub pr_state: Option<PrState>,
    /// False while the async PR fetch has not yet returned for this
    /// row. Drives the placeholder status label and forces the row to
    /// the Kept section so an unconfirmed default cannot suggest
    /// deletion.
    pub pr_loaded: bool,
}

/// Compose the partitioned row list from worktree discovery output,
/// PR fetch result, and the live session list. Pure function — no I/O.
pub fn build_clean_rows(
    inputs: Vec<CleanRowInput>,
    sessions: &[Session],
    now: DateTime<Utc>,
    timeout: Duration,
) -> Vec<CleanRow> {
    // Canonicalize cwds once for the whole batch so the has_active /
    // updated_at / session-children passes do not each re-canonicalize.
    let canonical_sessions: Vec<(PathBuf, &Session)> = sessions
        .iter()
        .map(|s| (canonicalize_or_self(&s.cwd), s))
        .collect();
    inputs
        .into_iter()
        .map(|input| {
            let CleanRowInput {
                row,
                merge_status,
                pr_number,
                pr_state,
                pr_loaded,
            } = input;

            let has_active = canonical_sessions
                .iter()
                .filter(|(c, _)| c.starts_with(&row.path))
                .any(|(_, s)| is_session_active(s, &NoActivityProbe, now, timeout));

            let pr_merged = matches!(merge_status, Some(MergeStatus::Merged { .. }));
            let status_label = if pr_loaded {
                format_status_label(pr_state, pr_number, has_active)
            } else {
                PR_FETCHING_LABEL.to_string()
            };

            // Default partition: PR merged AND no active session → delete.
            // Everything else (including not-yet-loaded rows) → keep.
            let section = if pr_loaded && pr_merged && !has_active {
                CleanSection::ToDelete
            } else {
                CleanSection::Kept
            };

            let updated_at = canonical_sessions
                .iter()
                .filter(|(c, _)| c.starts_with(&row.path))
                .map(|(_, s)| s.updated_at)
                .max();

            let session_children =
                sessions_under_worktree_from_canonical(&row.path, &canonical_sessions);

            CleanRow {
                repo: row.repo,
                branch: row.branch,
                name: row.name,
                path: row.path,
                session_count: row.session_count,
                has_active,
                updated_at,
                status_label,
                pr_merged,
                section,
                sessions: session_children,
            }
        })
        .collect()
}

fn format_status_label(
    pr_state: Option<PrState>,
    pr_number: Option<u64>,
    has_active: bool,
) -> String {
    let core = match (pr_state, pr_number) {
        (Some(PrState::Merged), Some(n)) => format!("PR #{n} merged"),
        (Some(PrState::Open), Some(n)) => format!("PR #{n} open"),
        (Some(PrState::Closed), Some(n)) => format!("PR #{n} closed"),
        (Some(PrState::Merged), None) => "PR merged".to_string(),
        (Some(PrState::Open), None) => "PR open".to_string(),
        (Some(PrState::Closed), None) => "PR closed".to_string(),
        (None, _) => "no PR".to_string(),
    };
    if has_active {
        format!("{core} · active")
    } else {
        core
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::types::{Session, SessionStatus};
    use chrono::Utc;
    use rstest::{fixture, rstest};
    use std::collections::BTreeSet;
    use std::path::Path;

    fn wt_row(repo: &str, branch: &str, name: &str, path: &str) -> WorktreeRow {
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

    fn merged_input(
        repo: &str,
        branch: &str,
        name: &str,
        path: &str,
        number: u64,
    ) -> CleanRowInput {
        CleanRowInput {
            row: wt_row(repo, branch, name, path),
            merge_status: Some(MergeStatus::Merged {
                reason: format!("#{number} merged"),
            }),
            pr_number: Some(number),
            pr_state: Some(PrState::Merged),
            pr_loaded: true,
        }
    }

    fn open_input(repo: &str, branch: &str, name: &str, path: &str, number: u64) -> CleanRowInput {
        CleanRowInput {
            row: wt_row(repo, branch, name, path),
            merge_status: Some(MergeStatus::NotMerged {
                reason: format!("#{number} open"),
            }),
            pr_number: Some(number),
            pr_state: Some(PrState::Open),
            pr_loaded: true,
        }
    }

    fn no_pr_input(repo: &str, branch: &str, name: &str, path: &str) -> CleanRowInput {
        CleanRowInput {
            row: wt_row(repo, branch, name, path),
            merge_status: None,
            pr_number: None,
            pr_state: None,
            pr_loaded: true,
        }
    }

    fn pending_input(repo: &str, branch: &str, name: &str, path: &str) -> CleanRowInput {
        CleanRowInput {
            row: wt_row(repo, branch, name, path),
            merge_status: None,
            pr_number: None,
            pr_state: None,
            pr_loaded: false,
        }
    }

    #[fixture]
    fn view_with_rows() -> CleanView {
        let mut v = CleanView::new();
        let rows = build_clean_rows(
            vec![
                merged_input("repo1", "feat/a", "feat-a", "/tmp/r1/wt-a", 1),
                open_input("repo1", "fix/b", "fix-b", "/tmp/r1/wt-b", 2),
                no_pr_input("repo2", "trunk", "trunk", "/tmp/r2/wt-c"),
            ],
            &[],
            Utc::now(),
            Duration::from_secs(60),
        );
        v.set_rows(rows);
        v
    }

    #[rstest]
    fn merged_row_defaults_to_delete(view_with_rows: CleanView) {
        let merged = view_with_rows
            .rows()
            .iter()
            .find(|r| r.name == "feat-a")
            .expect("merged row");
        assert_eq!(merged.section, CleanSection::ToDelete);
        assert_eq!(merged.status_label, "PR #1 merged");
    }

    #[rstest]
    #[case::open("fix-b", CleanSection::Kept, "PR #2 open")]
    #[case::no_pr("trunk", CleanSection::Kept, "no PR")]
    fn non_merged_rows_default_to_kept(
        view_with_rows: CleanView,
        #[case] name: &str,
        #[case] expected_section: CleanSection,
        #[case] expected_label: &str,
    ) {
        let row = view_with_rows
            .rows()
            .iter()
            .find(|r| r.name == name)
            .expect("row");
        assert_eq!(row.section, expected_section);
        assert_eq!(row.status_label, expected_label);
    }

    #[rstest]
    fn list_entries_groups_by_section_then_repo(view_with_rows: CleanView) {
        let entries = view_with_rows.list_entries();
        // Sections: ToDelete (1 row, repo1) + Kept (2 rows, repo1 + repo2)
        // = 2 section headers + 2 repo headers (ToDelete:repo1, Kept:repo1) + 1 repo header (Kept:repo2) + 3 rows
        // = 2 + 3 + 3 = 8
        assert_eq!(entries.len(), 8);
        assert!(matches!(
            entries[0],
            CleanListEntry::SectionHeader {
                section: CleanSection::ToDelete,
                count: 1
            }
        ));
        assert!(matches!(&entries[1], CleanListEntry::RepoHeader(r) if r == "repo1"));
        assert!(matches!(&entries[2], CleanListEntry::Row(r) if r.name == "feat-a"));
        assert!(matches!(
            entries[3],
            CleanListEntry::SectionHeader {
                section: CleanSection::Kept,
                count: 2
            }
        ));
    }

    #[rstest]
    fn selectable_indices_skip_headers(view_with_rows: CleanView) {
        let entries = view_with_rows.list_entries();
        let sel = view_with_rows.selectable_indices();
        for &i in &sel {
            assert!(matches!(entries[i], CleanListEntry::Row(_)));
        }
        assert_eq!(sel.len(), 3);
    }

    #[rstest]
    fn select_next_wraps_through_rows(mut view_with_rows: CleanView) {
        let initial = view_with_rows.list_state.selected().expect("initial");
        view_with_rows.select_next();
        let next = view_with_rows.list_state.selected().expect("next");
        assert!(next > initial);
        // Wrap eventually.
        view_with_rows.select_next();
        view_with_rows.select_next();
        let after_wrap = view_with_rows.list_state.selected().expect("wrap");
        assert_eq!(after_wrap, initial);
    }

    #[rstest]
    fn toggle_moves_row_between_sections(mut view_with_rows: CleanView) {
        // Cursor starts on the first ToDelete row ("feat-a").
        let before = view_with_rows.selected_row().expect("row");
        assert_eq!(before.section, CleanSection::ToDelete);

        assert!(view_with_rows.toggle_selected_section());

        let after = view_with_rows.selected_row().expect("row");
        assert_eq!(after.path, before.path);
        assert_eq!(after.section, CleanSection::Kept);
    }

    #[rstest]
    fn to_delete_paths_reflect_partition(view_with_rows: CleanView) {
        let paths = view_with_rows.to_delete_paths();
        assert_eq!(paths, vec![PathBuf::from("/tmp/r1/wt-a")]);
    }

    #[rstest]
    fn summary_counts_active_excluded(mut view_with_rows: CleanView) {
        // Force one Kept row to look active so summary sees it.
        if let CleanLoadState::Ready(rows) = &mut view_with_rows.state
            && let Some(r) = rows.iter_mut().find(|r| r.name == "trunk")
        {
            r.has_active = true;
        }
        let (to_delete, kept_active) = view_with_rows.summary();
        assert_eq!(to_delete, 1);
        assert_eq!(kept_active, 1);
    }

    #[rstest]
    fn remove_paths_drops_matching_rows(mut view_with_rows: CleanView) {
        view_with_rows.remove_paths(&[PathBuf::from("/tmp/r1/wt-a")]);
        assert!(
            view_with_rows
                .rows()
                .iter()
                .all(|r| r.path != Path::new("/tmp/r1/wt-a"))
        );
        assert_eq!(view_with_rows.rows().len(), 2);
    }

    fn session(id: &str, cwd: PathBuf) -> Session {
        Session {
            session_id: id.to_string(),
            cwd,
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: BTreeSet::new(),
        }
    }

    #[rstest]
    fn list_entries_nest_sessions_under_worktree_row_and_skip_toggle() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let wt = tmp.path().join("wt");
        std::fs::create_dir_all(&wt).expect("mkdir");

        let row = WorktreeRow {
            repo: "r".to_string(),
            branch: "b".to_string(),
            name: "wt".to_string(),
            path: super::super::worktree_view::canonicalize_or_self(&wt),
            session_count: 1,
            has_active: false,
            sessions: Vec::new(),
        };
        let input = CleanRowInput {
            row,
            merge_status: Some(MergeStatus::Merged {
                reason: "#1 merged".to_string(),
            }),
            pr_number: Some(1),
            pr_state: Some(PrState::Merged),
        };
        // Ended sessions are not "active", so the merged row stays in
        // ToDelete while still surfacing the (defunct) session as a
        // tree child.
        let mut session_obj = session("s1", wt.clone());
        session_obj.status = SessionStatus::Ended;
        let rows = build_clean_rows(
            vec![input],
            &[session_obj],
            Utc::now(),
            Duration::from_secs(60),
        );
        let mut v = CleanView::new();
        v.set_rows(rows);

        let entries = v.list_entries();
        // SectionHeader(ToDelete) + RepoHeader(r) + Row + Session +
        // SectionHeader(Kept, count=0).
        assert_eq!(entries.len(), 5);
        assert!(matches!(entries[2], CleanListEntry::Row(_)));
        assert!(matches!(entries[3], CleanListEntry::Session(_)));

        // Both Row and Session rows are selectable; the cursor lands on
        // the Row first so Enter still defaults to toggle.
        assert_eq!(v.selectable_indices(), vec![2, 3]);
        assert_eq!(v.list_state.selected(), Some(2));

        // Step onto the session row: selected_session_child returns it,
        // selected_row returns None, and toggle_selected_section is a
        // no-op on this row.
        v.select_next();
        assert_eq!(v.list_state.selected(), Some(3));
        let child = v.selected_session_child().expect("child");
        assert_eq!(child.session_id, "s1");
        assert!(v.selected_row().is_none());

        // Toggle attempted on a session row must NOT move the worktree
        // between sections — the underlying row is still ToDelete.
        v.toggle_selected_section();
        assert_eq!(
            v.rows().iter().map(|r| r.section).collect::<Vec<_>>(),
            vec![CleanSection::ToDelete],
        );
    }

    #[rstest]
    fn merged_with_active_session_defaults_to_kept() {
        // A merged PR row whose worktree still has an active session must
        // default to Kept so we do not blow away in-flight work.
        let tmp = tempfile::tempdir().expect("tempdir");
        let wt = tmp.path().join("wt");
        std::fs::create_dir_all(&wt).expect("mkdir");

        let row = WorktreeRow {
            repo: "r".to_string(),
            branch: "b".to_string(),
            name: "wt".to_string(),
            path: super::super::worktree_view::canonicalize_or_self(&wt),
            session_count: 1,
            has_active: false,
            sessions: Vec::new(),
        };
        let input = CleanRowInput {
            row,
            merge_status: Some(MergeStatus::Merged {
                reason: "#9 merged".to_string(),
            }),
            pr_number: Some(9),
            pr_state: Some(PrState::Merged),
            pr_loaded: true,
        };

        let sessions = vec![session("s1", wt.clone())];
        let rows = build_clean_rows(vec![input], &sessions, Utc::now(), Duration::from_secs(60));

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].section, CleanSection::Kept);
        assert!(rows[0].has_active);
        assert!(rows[0].status_label.contains("active"));
    }

    #[rstest]
    fn pending_row_defaults_to_kept_with_fetching_label() {
        let rows = build_clean_rows(
            vec![pending_input("r1", "feat/a", "feat-a", "/tmp/r1/wt-a")],
            &[],
            Utc::now(),
            Duration::from_secs(60),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].section, CleanSection::Kept);
        assert_eq!(rows[0].status_label, PR_FETCHING_LABEL);
        assert!(!rows[0].pr_merged);
    }

    #[rstest]
    fn apply_pr_results_replaces_labels_and_repartitions() {
        let mut v = CleanView::new();
        let initial = build_clean_rows(
            vec![
                pending_input("r1", "feat/a", "feat-a", "/tmp/r1/wt-a"),
                pending_input("r1", "fix/b", "fix-b", "/tmp/r1/wt-b"),
            ],
            &[],
            Utc::now(),
            Duration::from_secs(60),
        );
        v.set_initial_rows(initial);
        assert_eq!(v.pr_fetch, PrFetchStatus::Loading);
        assert!(v.rows().iter().all(|r| r.status_label == PR_FETCHING_LABEL));
        assert!(v.rows().iter().all(|r| r.section == CleanSection::Kept));

        let resolved = build_clean_rows(
            vec![
                merged_input("r1", "feat/a", "feat-a", "/tmp/r1/wt-a", 1),
                open_input("r1", "fix/b", "fix-b", "/tmp/r1/wt-b", 2),
            ],
            &[],
            Utc::now(),
            Duration::from_secs(60),
        );
        v.apply_pr_results(resolved);

        assert_eq!(v.pr_fetch, PrFetchStatus::Done);
        let by_name: std::collections::HashMap<_, _> = v
            .rows()
            .iter()
            .map(|r| (r.name.clone(), r.clone()))
            .collect();
        assert_eq!(by_name["feat-a"].section, CleanSection::ToDelete);
        assert_eq!(by_name["feat-a"].status_label, "PR #1 merged");
        assert_eq!(by_name["fix-b"].section, CleanSection::Kept);
        assert_eq!(by_name["fix-b"].status_label, "PR #2 open");
    }

    #[rstest]
    fn toggle_blocked_while_pr_loading() {
        let mut v = CleanView::new();
        v.set_initial_rows(build_clean_rows(
            vec![pending_input("r1", "feat/a", "feat-a", "/tmp/r1/wt-a")],
            &[],
            Utc::now(),
            Duration::from_secs(60),
        ));
        assert!(!v.toggle_selected_section());
        assert_eq!(v.rows()[0].section, CleanSection::Kept);
    }

    #[rstest]
    fn toggle_works_after_pr_fetch_done() {
        let mut v = CleanView::new();
        v.set_initial_rows(build_clean_rows(
            vec![pending_input("r1", "feat/a", "feat-a", "/tmp/r1/wt-a")],
            &[],
            Utc::now(),
            Duration::from_secs(60),
        ));
        v.apply_pr_results(build_clean_rows(
            vec![open_input("r1", "feat/a", "feat-a", "/tmp/r1/wt-a", 1)],
            &[],
            Utc::now(),
            Duration::from_secs(60),
        ));
        assert!(v.toggle_selected_section());
        assert_eq!(v.rows()[0].section, CleanSection::ToDelete);
    }

    #[rstest]
    fn set_failed_after_initial_rows_rewrites_pending_labels() {
        let mut v = CleanView::new();
        v.set_initial_rows(build_clean_rows(
            vec![pending_input("r1", "feat/a", "feat-a", "/tmp/r1/wt-a")],
            &[],
            Utc::now(),
            Duration::from_secs(60),
        ));
        v.set_failed("boom".to_string());
        assert_eq!(v.pr_fetch, PrFetchStatus::Failed("boom".to_string()));
        assert_eq!(v.rows()[0].status_label, PR_FETCH_FAILED_LABEL);
        assert!(matches!(v.state, CleanLoadState::Ready(_)));
    }
}
