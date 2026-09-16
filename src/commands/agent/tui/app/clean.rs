use std::path::PathBuf;

use super::super::clean_progress::CleanLogEvent;
use super::super::worktree_view::WorktreeRow;
use super::{App, View};

impl App {
    /// Snapshot of the currently discovered worktree rows, suitable for
    /// driving the clean view's PR fetch. Returns an empty vec while the
    /// discovery is still loading or failed.
    pub fn worktree_rows_snapshot(&self) -> Vec<WorktreeRow> {
        match &self.worktree_view.state {
            super::super::worktree_view::WorktreeLoadState::Loaded(rows) => rows.clone(),
            _ => Vec::new(),
        }
    }

    /// Switch into the clean view. Records the current view so the user
    /// can return via Esc/n/q, then seeds the row list synchronously
    /// from the worktree snapshot so the user sees rows immediately
    /// while the async PR fetch runs.
    ///
    /// Returns true when the worktree snapshot was non-empty and the
    /// caller should kick off the PR fetch. If false, the clean view
    /// stays in `LoadingPr` and seeding is deferred to
    /// [`Self::seed_clean_view_if_pending`] once worktrees arrive.
    pub fn enter_clean_view(&mut self) -> bool {
        if self.view == View::Clean {
            return false;
        }
        self.clean_return_view = self.view;
        self.view = View::Clean;
        self.clean_view.reset();
        self.seed_clean_view_if_pending()
    }

    /// Seed the clean view from the current worktree snapshot when it
    /// is still waiting for its initial rows. Returns true when seeding
    /// actually happened so the caller can kick off the PR fetch.
    pub fn seed_clean_view_if_pending(&mut self) -> bool {
        if self.view != View::Clean
            || !matches!(
                self.clean_view.state,
                super::super::clean_view::CleanLoadState::LoadingPr
            )
        {
            return false;
        }
        // Distinguish "discovery still running" from "discovery done
        // with zero worktrees" — the latter must transition out of
        // LoadingPr so the empty-list placeholder renders instead of a
        // permanent "Loading worktrees..." banner.
        let super::super::worktree_view::WorktreeLoadState::Loaded(rows) =
            &self.worktree_view.state
        else {
            return false;
        };
        if rows.is_empty() {
            self.clean_view.set_initial_rows(Vec::new());
            self.clean_view.pr_fetch = super::super::clean_view::PrFetchStatus::Done;
            return false;
        }
        let initial =
            super::super::pr_fetch::build_initial_clean_rows(rows.clone(), &self.sessions);
        self.clean_view.set_initial_rows(initial);
        true
    }

    /// Leave the clean view without acting on the partition; returns
    /// to whichever view the user came from.
    pub fn exit_clean_view(&mut self) {
        self.view = self.clean_return_view;
    }

    /// Install fully PR-enriched rows directly. Used by tests; the
    /// production code path goes through [`Self::apply_clean_pr_results`]
    /// instead so the placeholder list set up in `enter_clean_view`
    /// merges with the async result.
    #[cfg(test)]
    pub fn set_clean_rows(&mut self, mut rows: Vec<super::super::clean_view::CleanRow>) {
        rows = self.filter_already_cleaned(rows);
        self.clean_view.set_rows(rows);
    }

    /// Merge PR-enriched rows returned by the async fetch into the
    /// placeholder list seeded on entry. Drops any path that an
    /// in-flight cleanup has already removed.
    pub fn apply_clean_pr_results(&mut self, rows: Vec<super::super::clean_view::CleanRow>) {
        let rows = self.filter_already_cleaned(rows);
        self.clean_view.apply_pr_results(rows);
    }

    fn filter_already_cleaned(
        &self,
        mut rows: Vec<super::super::clean_view::CleanRow>,
    ) -> Vec<super::super::clean_view::CleanRow> {
        if let Some(progress) = &self.clean_progress {
            let deleted: Vec<PathBuf> = progress
                .confirmed_deleted
                .iter()
                .map(PathBuf::from)
                .collect();
            if !deleted.is_empty() {
                rows.retain(|r| !deleted.iter().any(|d| d == &r.path));
            }
        }
        rows
    }

    /// Mark the PR fetch as failed; the clean view shows the error and
    /// the user can press n/Esc to back out.
    pub fn set_clean_failed(&mut self, error: String) {
        self.clean_view.set_failed(error);
    }

    /// Fold a batch of JSONL events from the detached child into the
    /// live progress state and drop any worktree rows that the child
    /// confirmed deleted.
    pub fn apply_clean_log_events(&mut self, events: &[CleanLogEvent]) {
        let Some(progress) = self.clean_progress.as_mut() else {
            return;
        };
        for event in events {
            progress.apply(event);
        }
        // Drop deleted rows from both lists so the cleanup is reflected
        // without a fresh discovery pass.
        let deleted: Vec<PathBuf> = progress.deleted_paths.iter().map(PathBuf::from).collect();
        if !deleted.is_empty() {
            if let super::super::worktree_view::WorktreeLoadState::Loaded(rows) =
                &mut self.worktree_view.state
            {
                rows.retain(|r| !deleted.iter().any(|d| d == &r.path));
            }
            self.clean_view.remove_paths(&deleted);
            // Mark the deleted paths as drained so we do not pop the
            // same rows twice on the next batch.
            progress.deleted_paths.clear();
        }
    }

    /// Dismiss the bottom-bar summary. Called on the first key press
    /// after the detached child reports `Done` so the stale "Cleaned
    /// X, failed Y" line does not linger.
    pub fn clear_clean_progress(&mut self) {
        self.clean_progress = None;
    }
}
