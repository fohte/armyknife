use std::path::Path;

use crate::commands::cc::store;
use crate::infra::tmux;

use super::worktree::resolve_worktree_root;
use super::{App, AppMode};

impl App {
    /// Enters confirm-delete mode for the currently selected session.
    ///
    /// If the session is the last one in its worktree, records the worktree
    /// root so a single `y` will delete both the session and the worktree
    /// (branch, tmux windows, worktree dir) in one confirmation.
    pub fn request_delete(&mut self) {
        if let Some(session) = self.selected_session() {
            let session_id = session.session_id.clone();
            let cwd = session.cwd.clone();
            let is_alive = session
                .tmux_info
                .as_ref()
                .is_some_and(|info| tmux::is_pane_alive(&info.pane_id));

            let worktree_cleanup = resolve_worktree_root(&cwd).filter(|wt_root| {
                !self
                    .sessions
                    .iter()
                    .any(|s| s.session_id != session_id && s.cwd.starts_with(wt_root))
            });

            self.mode = AppMode::Confirm {
                session_id,
                is_alive,
                worktree_cleanup,
            };
        }
    }

    /// Executes the confirmed delete action for the selected session.
    /// If the session is alive, sends SIGTERM to the pane process first.
    ///
    /// When `worktree_cleanup` is `Some`, also removes the worktree, its
    /// branch, associated tmux windows, and any remaining session files
    /// inside the worktree.
    pub fn confirm_delete(&mut self) -> anyhow::Result<()> {
        let current_selection = self.list_state.selected();
        let (session_id, is_alive, worktree_cleanup) = match &self.mode {
            AppMode::Confirm {
                session_id,
                is_alive,
                worktree_cleanup,
            } => (session_id.clone(), *is_alive, worktree_cleanup.clone()),
            _ => return Ok(()),
        };

        if is_alive
            && let Some(session) = self.sessions.iter().find(|s| s.session_id == session_id)
            && let Some(ref tmux_info) = session.tmux_info
        {
            tmux::send_sigterm_to_pane(&tmux_info.pane_id);
        }

        store::delete_session(&session_id)?;
        self.remove_session(&session_id);
        self.mode = AppMode::Normal;

        // Re-verify siblings at delete time: between request_delete and here
        // the user may have sat on the confirm prompt while a new session was
        // created inside the same worktree.
        let cleanup_result = if let Some(worktree_root) = worktree_cleanup
            && !self.has_session_in_worktree(&worktree_root)
        {
            use crate::shared::cleanup;
            cleanup::cleanup_worktree_resources(&worktree_root)
        } else {
            Ok(Default::default())
        };

        if let Ok(ref result) = cleanup_result
            && let Some(ref wt_root) = result.worktree_root
        {
            let to_remove: Vec<String> = self
                .sessions
                .iter()
                .filter(|s| s.cwd.starts_with(wt_root))
                .map(|s| s.session_id.clone())
                .collect();
            for id in &to_remove {
                self.remove_session(id);
            }
        }

        // Always refresh, even if cleanup below failed: the session itself is
        // already gone from disk and memory, so list_state must not point at
        // the stale index.
        self.refresh_after_mutation(current_selection);
        cleanup_result.map(|_| ())
    }

    fn has_session_in_worktree(&self, worktree_root: &Path) -> bool {
        self.sessions
            .iter()
            .any(|s| s.cwd.starts_with(worktree_root))
    }

    /// Cancels the confirm-delete dialog.
    pub fn cancel_confirm(&mut self) {
        self.mode = AppMode::Normal;
    }

    /// Re-sorts sessions, rebuilds caches, reapplies filters, and restores
    /// selection.
    fn refresh_after_mutation(&mut self, previous_selection: Option<usize>) {
        store::sort_sessions(&mut self.sessions);
        self.rebuild_title_cache();
        if self.searchable_text_cache.is_some() {
            self.update_searchable_text_cache();
        }
        self.apply_filter();
        self.resync_selection(previous_selection, None);
    }
}
