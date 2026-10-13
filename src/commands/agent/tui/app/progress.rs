use std::collections::HashMap;

use super::super::progress::{
    FetchedProgress, ProgressRow, build_progress_rows, refresh_rows_from_sessions,
};
use super::super::progress_batches::{
    ImplementationPlan, build_progress_rows as build_checklist_progress_rows,
};
use super::super::progress_fetch::direct_delegates;
use super::App;
use crate::commands::agent::types::Session;

#[derive(Clone)]
pub(super) struct CachedProgress {
    rows: Vec<ProgressRow>,
    error: Option<String>,
    implementation: Option<ImplementationPlan>,
}

impl App {
    pub(in crate::commands::agent::tui) fn begin_progress_refresh(&mut self, session_id: &str) {
        self.refresh_local_progress(session_id, None);
    }

    pub(in crate::commands::agent::tui) fn refresh_progress_from_sessions(
        &mut self,
        session_id: &str,
    ) {
        self.refresh_local_progress(session_id, None);
    }

    pub(in crate::commands::agent::tui) fn apply_progress_result(
        &mut self,
        session_id: &str,
        result: Result<FetchedProgress, String>,
    ) {
        match result {
            Ok(progress) if progress.selected_session_id == session_id => {
                let Some(selected) = self.session_by_id(session_id) else {
                    return;
                };
                let has_delegates = !progress.delegated.is_empty();
                let implementation = progress.implementation.clone();
                let delegated = progress
                    .delegated
                    .into_iter()
                    .map(|mut fetched| {
                        if let Some(session) = self.session_by_id(&fetched.session.session_id) {
                            fetched.session = session;
                        }
                        fetched
                    })
                    .collect::<Vec<_>>();
                let rows = if let Some(plan) = implementation.as_ref() {
                    self.checklist_rows(
                        &selected,
                        &delegated
                            .iter()
                            .map(|delegate| delegate.session.clone())
                            .collect::<Vec<_>>(),
                        plan,
                    )
                } else {
                    build_progress_rows(&selected, &delegated, &self.agent_config)
                };
                let error = (has_delegates || implementation.is_some())
                    .then_some(progress.github_error)
                    .flatten();
                self.store_progress_snapshot(
                    session_id,
                    CachedProgress {
                        rows,
                        error,
                        implementation,
                    },
                );
            }
            Ok(_) => {}
            Err(error) => self.refresh_local_progress(session_id, Some(error)),
        }
    }

    fn existing_progress_rows(&self, session_id: &str) -> Vec<ProgressRow> {
        self.progress_cache
            .get(session_id)
            .map(|snapshot| snapshot.rows.clone())
            .unwrap_or_default()
    }

    fn session_by_id(&self, session_id: &str) -> Option<Session> {
        self.sessions
            .iter()
            .find(|session| session.session_id == session_id)
            .cloned()
    }

    fn refresh_local_progress(&mut self, session_id: &str, error: Option<String>) {
        let Some(selected) = self.session_by_id(session_id) else {
            return;
        };
        let rows = self.existing_progress_rows(session_id);
        let mut snapshot = self.local_progress_snapshot(&selected, &rows);
        snapshot.error = error.or(snapshot.error);
        self.store_progress_snapshot(session_id, snapshot);
    }

    fn local_progress_snapshot(
        &self,
        selected: &Session,
        existing_rows: &[ProgressRow],
    ) -> CachedProgress {
        let delegated = direct_delegates(self.sessions.clone(), &selected.session_id);
        let implementation = self
            .progress_cache
            .get(&selected.session_id)
            .and_then(|snapshot| snapshot.implementation.clone());
        let rows = if let Some(plan) = implementation.as_ref() {
            self.checklist_rows(selected, &delegated, plan)
        } else {
            refresh_rows_from_sessions(selected, &delegated, existing_rows, &self.agent_config)
        };
        let error = if rows.len() > 1 {
            self.progress_cache
                .get(&selected.session_id)
                .and_then(|snapshot| snapshot.error.clone())
        } else {
            None
        };
        CachedProgress {
            rows,
            error,
            implementation,
        }
    }

    fn store_progress_snapshot(&mut self, session_id: &str, snapshot: CachedProgress) {
        self.progress_cache
            .insert(session_id.to_string(), snapshot.clone());
        if self.selected_session_id() != Some(session_id) {
            return;
        }
        if self
            .initial_progress_selection
            .as_ref()
            .is_some_and(|(parent_id, _)| parent_id != session_id)
        {
            self.initial_progress_selection = None;
        }
        if self.progress_session_id.as_deref() != Some(session_id) {
            self.progress_implementation_expanded = true;
            self.progress_batch_expanded.clear();
        }
        if let Some((_, delegate_id)) = self
            .initial_progress_selection
            .as_ref()
            .filter(|(parent_id, _)| parent_id == session_id)
        {
            self.progress_implementation_expanded = true;
            if let Some(batch_id) = snapshot
                .implementation
                .as_ref()
                .and_then(|plan| plan.batch_id_for_delegate(delegate_id))
            {
                self.progress_batch_expanded
                    .insert(batch_id.to_string(), true);
            }
        }
        let initial_selection_index =
            self.initial_progress_selection
                .as_ref()
                .and_then(|(_, delegate_id)| {
                    snapshot
                        .rows
                        .iter()
                        .position(|row| row.selection_key() == Some(delegate_id.as_str()))
                });
        let (selected_index, selected_key) = if let Some(index) = initial_selection_index {
            let key = snapshot.rows[index].selection_key().map(str::to_owned);
            self.initial_progress_selection = None;
            (index, key)
        } else if self.progress_session_id.as_deref() == Some(session_id) {
            self.current_progress_selection()
        } else {
            (0, None)
        };
        self.progress_session_id = Some(session_id.to_string());
        self.progress_error = snapshot.error.clone();
        self.replace_progress_rows(snapshot.rows, selected_key, selected_index);
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
        let Some(plan) = self.current_progress_implementation().cloned() else {
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
        let use_current_expansion = self.progress_session_id.as_deref()
            == Some(selected.session_id.as_str())
            && self.selected_session_id() == Some(selected.session_id.as_str());
        let mut expanded_batches = if use_current_expansion {
            self.progress_batch_expanded.clone()
        } else {
            HashMap::new()
        };
        let initial_target = self
            .initial_progress_selection
            .as_ref()
            .filter(|(parent_id, _)| parent_id == &selected.session_id)
            .map(|(_, delegate_id)| delegate_id);
        if let Some(target) = initial_target
            && let Some(batch_id) = plan.batch_id_for_delegate(target)
        {
            expanded_batches.insert(batch_id.to_string(), true);
        }
        build_checklist_progress_rows(
            selected,
            delegated,
            plan,
            initial_target.is_some()
                || !use_current_expansion
                || self.progress_implementation_expanded,
            &expanded_batches,
            &self.agent_config,
        )
    }

    fn current_progress_implementation(&self) -> Option<&ImplementationPlan> {
        self.progress_session_id
            .as_deref()
            .and_then(|session_id| self.progress_cache.get(session_id))
            .and_then(|snapshot| snapshot.implementation.as_ref())
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

enum FoldTarget {
    Implementation,
    Batch(String),
}
