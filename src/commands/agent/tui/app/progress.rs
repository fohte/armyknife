use super::super::progress::{
    FetchedProgress, ProgressRow, build_progress_rows, refresh_rows_from_sessions,
};
use super::super::progress_batches::ImplementationPlan;
use super::super::progress_batches::build_progress_rows as build_checklist_progress_rows;
use super::super::progress_fetch::direct_delegates;
use super::{App, Session};

enum FoldTarget {
    Implementation,
    Batch(String),
}

impl App {
    pub(in crate::commands::agent::tui) fn begin_progress_refresh(&mut self, session_id: &str) {
        let Some(session) = self
            .selected_session()
            .filter(|session| session.session_id == session_id)
            .cloned()
        else {
            return;
        };
        let work_row = build_progress_rows(&session, &[], &self.agent_config)
            .into_iter()
            .next();
        if self.progress_session_id.as_deref() != Some(session_id) {
            self.progress_rows = work_row.into_iter().collect();
            self.progress_checklist = None;
            self.progress_implementation_expanded = true;
            self.progress_batch_expanded.clear();
            self.progress_list_state.select(Some(0));
            self.progress_table_state.select(None);
            self.progress_session_id = Some(session_id.to_string());
        } else if let Some(work_row) = work_row {
            if let Some(first_row) = self.progress_rows.first_mut() {
                *first_row = work_row;
            } else {
                self.progress_rows.push(work_row);
            }
        }
        self.progress_loading = true;
        self.progress_error = None;
    }

    pub(in crate::commands::agent::tui) fn refresh_progress_from_sessions(
        &mut self,
        session_id: &str,
    ) {
        let Some(selected) = self
            .selected_session()
            .filter(|session| session.session_id == session_id)
            .cloned()
        else {
            return;
        };
        let (selected_index, selected_key) = self.current_progress_selection();
        let delegated = direct_delegates(self.sessions.clone(), session_id);
        let rows = match self.progress_checklist.as_ref() {
            Some(plan) => self.checklist_rows(&selected, &delegated, plan),
            None => refresh_rows_from_sessions(
                &selected,
                &delegated,
                &self.progress_rows,
                &self.agent_config,
            ),
        };
        self.replace_progress_rows(rows, selected_key, selected_index);
    }

    pub(in crate::commands::agent::tui) fn apply_progress_result(
        &mut self,
        session_id: &str,
        result: Result<FetchedProgress, String>,
    ) {
        if self.selected_session_id() != Some(session_id) {
            return;
        }
        match result {
            Ok(progress) if progress.selected_session_id == session_id => {
                let Some(selected) = self.selected_session().cloned() else {
                    return;
                };
                let (selected_index, selected_key) = self.current_progress_selection();
                let delegated_sessions = progress
                    .delegated
                    .iter()
                    .map(|delegate| delegate.session.clone())
                    .collect::<Vec<_>>();
                self.progress_checklist = progress.implementation.clone();
                let rows = match progress.implementation.as_ref() {
                    Some(plan) => self.checklist_rows(&selected, &delegated_sessions, plan),
                    None => build_progress_rows(&selected, &progress.delegated, &self.agent_config),
                };
                self.replace_progress_rows(rows, selected_key, selected_index);
                self.progress_session_id = Some(session_id.to_string());
                self.progress_loading = false;
                self.progress_error = progress.github_error;
            }
            Ok(_) => {}
            Err(error) => {
                self.progress_loading = false;
                self.progress_error = Some(error);
            }
        }
    }

    pub(in crate::commands::agent::tui) fn select_progress_row(&mut self, delta: isize) {
        let selectable = self
            .progress_rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| row.selection_key().map(|_| index))
            .collect::<Vec<_>>();
        if selectable.is_empty() {
            return;
        }

        let current = self.progress_list_state.selected().unwrap_or(0);
        let position = selectable
            .iter()
            .position(|index| *index == current)
            .unwrap_or_else(|| if delta < 0 { selectable.len() - 1 } else { 0 });
        let next_position = position
            .saturating_add_signed(delta)
            .min(selectable.len() - 1);
        let next_index = selectable[next_position];
        self.progress_list_state.select(Some(next_index));
        self.progress_table_state.select(next_index.checked_sub(1));
    }

    pub(in crate::commands::agent::tui) fn set_progress_batch_collapsed(
        &mut self,
        collapsed: bool,
    ) {
        let (current_index, selected_key) = self.current_progress_selection();
        let target = if collapsed {
            self.progress_rows.get(current_index).and_then(|row| {
                if matches!(row, ProgressRow::Work(_)) {
                    Some(FoldTarget::Implementation)
                } else {
                    row.batch_id().map(|id| FoldTarget::Batch(id.to_string()))
                }
            })
        } else {
            self.progress_rows
                .iter()
                .enumerate()
                .filter_map(|(index, row)| match row {
                    ProgressRow::ImplementationHeader {
                        collapsed: true, ..
                    } => Some((index.abs_diff(current_index), FoldTarget::Implementation)),
                    ProgressRow::BatchHeader {
                        batch_id,
                        collapsed: true,
                        ..
                    } => Some((
                        index.abs_diff(current_index),
                        FoldTarget::Batch(batch_id.clone()),
                    )),
                    _ => None,
                })
                .min_by_key(|(distance, _)| *distance)
                .map(|(_, target)| target)
        };
        let Some(target) = target else {
            return;
        };
        match &target {
            FoldTarget::Implementation if self.progress_implementation_expanded == !collapsed => {
                return;
            }
            FoldTarget::Batch(batch_id)
                if self.progress_batch_expanded.get(batch_id).copied() == Some(!collapsed) =>
            {
                return;
            }
            FoldTarget::Implementation | FoldTarget::Batch(_) => {}
        }
        let Some(selected) = self
            .selected_session()
            .filter(|session| {
                self.progress_session_id.as_deref() == Some(session.session_id.as_str())
            })
            .cloned()
        else {
            return;
        };
        let Some(plan) = self.progress_checklist.clone() else {
            return;
        };

        match target {
            FoldTarget::Implementation => self.progress_implementation_expanded = !collapsed,
            FoldTarget::Batch(batch_id) => {
                self.progress_batch_expanded.insert(batch_id, !collapsed);
            }
        }
        let delegated = direct_delegates(self.sessions.clone(), &selected.session_id);
        let rows = self.checklist_rows(&selected, &delegated, &plan);
        self.replace_progress_rows(rows, selected_key, current_index);
    }

    fn current_progress_selection(&self) -> (usize, Option<String>) {
        let index = self.progress_list_state.selected().unwrap_or(0);
        let key = self
            .progress_rows
            .get(index)
            .and_then(ProgressRow::selection_key)
            .map(str::to_owned);
        (index, key)
    }

    fn checklist_rows(
        &self,
        selected: &Session,
        delegated: &[Session],
        plan: &ImplementationPlan,
    ) -> Vec<ProgressRow> {
        build_checklist_progress_rows(
            selected,
            delegated,
            plan,
            self.progress_implementation_expanded,
            &self.progress_batch_expanded,
            &self.agent_config,
        )
    }

    fn replace_progress_rows(
        &mut self,
        rows: Vec<ProgressRow>,
        selected_key: Option<String>,
        previous_index: usize,
    ) {
        let selected_index = selected_key
            .and_then(|key| {
                rows.iter()
                    .position(|row| row.selection_key() == Some(key.as_str()))
            })
            .or_else(|| {
                rows.iter()
                    .enumerate()
                    .filter(|(_, row)| row.selection_key().is_some())
                    .min_by_key(|(index, _)| index.abs_diff(previous_index))
                    .map(|(index, _)| index)
            })
            .unwrap_or(0);
        self.progress_rows = rows;
        self.progress_list_state.select(Some(selected_index));
        self.progress_table_state
            .select(selected_index.checked_sub(1));
    }
}
