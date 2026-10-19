use super::super::App;
use super::super::progress_cache::same_implementation_scope;
use super::super::progress_selection::{FoldTarget, ProgressFoldAction};

impl App {
    pub(in crate::commands::agent::tui) fn set_progress_batch_collapsed(
        &mut self,
        collapsed: bool,
    ) {
        let action = if collapsed {
            ProgressFoldAction::Collapse
        } else {
            ProgressFoldAction::Expand
        };
        self.change_selected_progress_fold(action);
    }

    pub(in crate::commands::agent::tui) fn toggle_selected_progress_heading(&mut self) -> bool {
        self.change_selected_progress_fold(ProgressFoldAction::Toggle)
    }

    fn change_selected_progress_fold(&mut self, action: ProgressFoldAction) -> bool {
        let current_index = self.progress_list_state.selected().unwrap_or(0);
        let Some(selected_row) = self.progress_rows.get(current_index) else {
            return false;
        };
        let Some((target, was_collapsed)) = FoldTarget::for_row(
            selected_row,
            action,
            self.progress_implementation_expanded,
            self.progress_unmatched_expanded,
        ) else {
            return false;
        };
        let collapsed = match action {
            ProgressFoldAction::Collapse => true,
            ProgressFoldAction::Expand => false,
            ProgressFoldAction::Toggle => !was_collapsed,
        };
        if was_collapsed == collapsed {
            return true;
        }
        let Some(selected) = self
            .selected_session()
            .filter(|session| {
                self.progress_session_id.as_deref() == Some(session.session_id.as_str())
            })
            .cloned()
        else {
            return false;
        };
        let Some(plan) = self.current_progress_implementation().cloned() else {
            return false;
        };

        match &target {
            FoldTarget::Implementation => self.progress_implementation_expanded = !collapsed,
            FoldTarget::Batch(batch_id) => {
                self.progress_batch_expanded
                    .insert(batch_id.clone(), !collapsed);
            }
            FoldTarget::UnmatchedDelegates => {
                self.progress_unmatched_expanded = !collapsed;
            }
        }
        let scope = self.progress_scope(&selected);
        let request_scope = scope.kind.fetch_scope();
        let cached_delegates = self
            .cached_progress(&selected.session_id)
            .filter(|cached| {
                same_implementation_scope(cached.request_scope.as_ref(), request_scope.as_ref())
            })
            .map(|cached| cached.delegated_sessions.as_slice())
            .unwrap_or_default();
        let delegated =
            self.delegates_with_missing_cached_sessions(&scope.children, cached_delegates);
        let rows = self.checklist_rows(&scope.work_sessions, &delegated, &plan);
        self.replace_progress_rows(rows, Some(target.selection_key()), current_index);
        true
    }
}
