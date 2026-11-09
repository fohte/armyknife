use super::App;
use crate::commands::agent::tui::progress::{
    CurrentPrStage, ProgressFetchState, ProgressOpenTarget, ProgressRow, ProgressSelectionKey,
    current_pr_stage, open_pr_url_target,
};
use crate::commands::agent::tui::session_rows::{SessionRow, SessionRowKey};

impl App {
    pub(in crate::commands::agent::tui) fn selected_row_key(&self) -> Option<&SessionRowKey> {
        self.list_state
            .selected()
            .and_then(|position| self.row_keys.get(position))
            .and_then(Option::as_ref)
    }

    pub(in crate::commands::agent::tui) fn selected_progress_source_id(&self) -> Option<String> {
        let selected_position = self.list_state.selected()?;
        if let Some(SessionRowKey::Progress {
            source_session_id, ..
        }) = self
            .row_keys
            .get(selected_position)
            .and_then(Option::as_ref)
        {
            return Some(source_session_id.clone());
        }

        let session_id = self.selected_session_id()?;
        self.progress_source_for_session_id(session_id)
    }

    pub(super) fn progress_source_for_session_id(&self, session_id: &str) -> Option<String> {
        let session = self.session_by_id(session_id)?;
        let selected_task_id = self
            .task_by_session
            .get(session_id)
            .map(|task| task.task_id.as_str());
        if let Some(current_source) = self.progress_session_id.as_deref()
            && (session.session_id == current_source
                || session
                    .ancestor_session_ids
                    .iter()
                    .any(|ancestor| ancestor == current_source))
            && (session.session_id == current_source
                || selected_task_id.is_none()
                || selected_task_id
                    == self
                        .task_by_session
                        .get(current_source)
                        .map(|task| task.task_id.as_str()))
        {
            return Some(current_source.to_string());
        }

        if selected_task_id.is_some() {
            return Some(session.session_id.clone());
        }

        session
            .ancestor_session_ids
            .iter()
            .rev()
            .find(|ancestor| {
                self.sessions
                    .iter()
                    .any(|candidate| candidate.session_id.as_str() == ancestor.as_str())
            })
            .cloned()
            .or_else(|| Some(session.session_id.clone()))
    }

    pub(super) fn progress_status_label(
        &self,
        source_session_id: &str,
        rows: &[ProgressRow],
    ) -> Option<String> {
        let state = self
            .progress_fetch_states
            .get(source_session_id)
            .cloned()
            .or_else(|| {
                self.cached_progress(source_session_id)
                    .filter(|snapshot| snapshot.fetched_at.is_some())
                    .map(|_| ProgressFetchState::Ready)
            });
        match state {
            Some(ProgressFetchState::Loading) => Some("Refreshing PR progress…".to_string()),
            Some(ProgressFetchState::Failed(error)) => {
                Some(format!("PR progress fetch failed: {error}"))
            }
            Some(ProgressFetchState::Ready)
                if self
                    .cached_progress(source_session_id)
                    .and_then(|snapshot| snapshot.error.as_ref())
                    .is_some() =>
            {
                self.cached_progress(source_session_id)
                    .and_then(|snapshot| snapshot.error.as_ref())
                    .map(|error| format!("PR status lookup failed: {error}"))
            }
            Some(ProgressFetchState::Ready)
                if !rows
                    .iter()
                    .any(|row| matches!(row, ProgressRow::ImplementationHeader { .. })) =>
            {
                Some("No implementation checklist".to_string())
            }
            Some(ProgressFetchState::Ready) => None,
            None => Some("PR progress not fetched".to_string()),
        }
    }

    pub(in crate::commands::agent::tui) fn toggle_inline_progress(&mut self) {
        let Some(source_session_id) = self.selected_progress_source_id() else {
            return;
        };
        let selected_key = self
            .list_state
            .selected()
            .and_then(|position| self.row_keys.get(position))
            .and_then(Option::as_ref)
            .cloned();
        let is_open_for_source =
            self.progress_inline_source_id.as_deref() == Some(source_session_id.as_str());
        if !is_open_for_source {
            let anchor = self.selection_anchor();
            let previous_source_id = self.progress_inline_source_id.clone();
            self.progress_inline_source_id = Some(source_session_id);
            if !self
                .build_display_rows()
                .iter()
                .any(|row| matches!(row, SessionRow::InlineProgress(_)))
            {
                self.progress_inline_source_id = previous_source_id;
                return;
            }
            self.restore_selection(anchor);
            return;
        }

        self.progress_inline_source_id = None;
        let restore_source = matches!(
            selected_key,
            Some(SessionRowKey::Progress {
                session_id: None,
                ..
            })
        );
        if restore_source {
            self.rebuild_row_order();
            if let Some(position) = self.position_of_task_row(&source_session_id) {
                self.list_state.select(Some(position));
            }
        } else {
            let anchor = self.selection_anchor();
            self.restore_selection(anchor);
        }
    }

    pub(in crate::commands::agent::tui) fn toggle_inline_progress_heading(&mut self) -> bool {
        let Some((source_session_id, selection_key)) = self.selected_inline_progress_key() else {
            return false;
        };
        let Some(index) = self
            .progress_rows
            .iter()
            .position(|row| row.selection_key().as_ref() == Some(&selection_key))
        else {
            return false;
        };
        if !matches!(
            self.progress_rows.get(index),
            Some(ProgressRow::ImplementationHeader { .. } | ProgressRow::BatchHeader { .. })
        ) {
            return false;
        }
        let anchor = self.selection_anchor();
        self.progress_list_state.select(Some(index));
        let changed = self.toggle_selected_progress_heading();
        if changed {
            self.progress_session_id = Some(source_session_id);
            self.restore_selection(anchor);
        }
        changed
    }

    pub(in crate::commands::agent::tui) fn set_inline_progress_heading_expanded(
        &mut self,
        expanded: bool,
    ) -> bool {
        let Some((source_session_id, selection_key)) = self.selected_inline_progress_key() else {
            return false;
        };
        let Some(index) = self
            .progress_rows
            .iter()
            .position(|row| row.selection_key().as_ref() == Some(&selection_key))
        else {
            return false;
        };
        let can_fold = match self.progress_rows.get(index) {
            Some(ProgressRow::ImplementationHeader { .. } | ProgressRow::BatchHeader { .. }) => {
                true
            }
            Some(
                ProgressRow::ChecklistPullRequest { .. } | ProgressRow::PlannedPullRequest { .. },
            ) => !expanded,
            _ => false,
        };
        if !can_fold {
            return false;
        }
        let collapsed_batch_id = if !expanded {
            match self.progress_rows.get(index) {
                Some(
                    ProgressRow::ChecklistPullRequest { batch_id, .. }
                    | ProgressRow::PlannedPullRequest { batch_id, .. },
                ) => Some(batch_id.clone()),
                _ => None,
            }
        } else {
            None
        };
        let anchor = self.selection_anchor();
        self.progress_list_state.select(Some(index));
        self.set_progress_batch_collapsed(!expanded);
        self.progress_session_id = Some(source_session_id.clone());
        self.restore_selection(anchor);
        if let Some(batch_id) = collapsed_batch_id
            && let Some(position) = self.row_keys.iter().position(|key| {
                matches!(
                    key,
                    Some(SessionRowKey::Progress {
                        source_session_id: source,
                        selection_key: ProgressSelectionKey::Batch(id),
                        ..
                    }) if source == &source_session_id && id == &batch_id
                )
            })
        {
            self.list_state.select(Some(position));
        }
        true
    }

    pub(in crate::commands::agent::tui) fn selected_progress_open_target(
        &self,
    ) -> Option<ProgressOpenTarget> {
        let selected_session = self.selected_session();
        if selected_session.is_some_and(|session| !session.crit_urls.is_empty())
            && self
                .selected_session_pr_stage()
                .is_some_and(CurrentPrStage::is_crit)
        {
            return None;
        }

        match self.selected_row_key()? {
            SessionRowKey::Progress { selection_key, .. } => self
                .progress_rows
                .iter()
                .find(|row| row.selection_key().as_ref() == Some(selection_key))
                .and_then(open_pr_url_target),
            SessionRowKey::Task(session_id) => self
                .progress_rows
                .iter()
                .find(|row| match row {
                    ProgressRow::PullRequest(row)
                    | ProgressRow::ChecklistPullRequest { row, .. } => {
                        row.session_id == *session_id
                    }
                    _ => false,
                })
                .and_then(open_pr_url_target)
                .or_else(|| self.worker_pr_open_target(session_id)),
        }
    }

    fn selected_session_pr_stage(&self) -> Option<CurrentPrStage> {
        let session = self.selected_session()?;
        self.worker_pr_stage_for_session(session).or_else(|| {
            self.progress_rows.iter().find_map(|row| {
                let row = match row {
                    ProgressRow::PullRequest(row)
                    | ProgressRow::ChecklistPullRequest { row, .. } => row,
                    _ => return None,
                };
                (row.session_id == session.session_id)
                    .then(|| current_pr_stage(row.stages, row.pr_stage_info.as_ref()))
            })
        })
    }

    fn selected_inline_progress_key(&self) -> Option<(String, ProgressSelectionKey)> {
        let key = self
            .list_state
            .selected()
            .and_then(|position| self.row_keys.get(position))
            .and_then(Option::as_ref)?;
        let SessionRowKey::Progress {
            source_session_id,
            selection_key,
            ..
        } = key
        else {
            return None;
        };
        Some((source_session_id.clone(), selection_key.clone()))
    }
}
