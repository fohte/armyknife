use std::collections::HashMap;

use super::super::progress::{
    FetchedProgress, ProgressRow, build_task_progress_rows, refresh_task_rows_from_sessions,
    table_selection_index,
};
use super::super::progress_batches::{
    ImplementationPlan, build_task_progress_rows as build_checklist_progress_rows,
};
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
        self.refresh_local_progress(session_id, None, false);
    }

    pub(in crate::commands::agent::tui) fn refresh_progress_from_sessions(
        &mut self,
        session_id: &str,
    ) {
        self.refresh_local_progress(session_id, None, false);
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
                let scope = self.progress_scope(&selected);
                let includes_fetched_progress = scope.kind.includes_fetched_progress();
                let implementation = includes_fetched_progress
                    .then_some(progress.implementation)
                    .flatten();
                let delegated = if includes_fetched_progress {
                    progress
                        .delegated
                        .into_iter()
                        .map(|mut fetched| {
                            if let Some(session) = self.session_by_id(&fetched.session.session_id) {
                                fetched.session = session;
                            }
                            fetched
                        })
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                let has_delegates = !delegated.is_empty();
                let rows = if let Some(plan) = implementation.as_ref() {
                    self.checklist_rows(
                        &scope.work_sessions,
                        &delegated
                            .iter()
                            .map(|delegate| delegate.session.clone())
                            .collect::<Vec<_>>(),
                        plan,
                    )
                } else {
                    build_task_progress_rows(&scope.work_sessions, &delegated, &self.agent_config)
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
                    true,
                );
            }
            Ok(_) => {}
            Err(error) => self.refresh_local_progress(session_id, Some(error), true),
        }
    }

    fn existing_progress_rows(&self, session_id: &str) -> Vec<ProgressRow> {
        self.progress_cache
            .get(session_id)
            .map(|snapshot| snapshot.rows.clone())
            .unwrap_or_default()
    }

    fn refresh_local_progress(
        &mut self,
        session_id: &str,
        error: Option<String>,
        resolve_initial_selection: bool,
    ) {
        let Some(selected) = self.session_by_id(session_id) else {
            return;
        };
        let rows = self.existing_progress_rows(session_id);
        let mut snapshot = self.local_progress_snapshot(&selected, &rows);
        snapshot.error = error.or(snapshot.error);
        self.store_progress_snapshot(session_id, snapshot, resolve_initial_selection);
    }

    fn local_progress_snapshot(
        &self,
        selected: &Session,
        existing_rows: &[ProgressRow],
    ) -> CachedProgress {
        let scope = self.progress_scope(selected);
        let implementation = self
            .progress_cache
            .get(&selected.session_id)
            .and_then(|snapshot| snapshot.implementation.clone());
        let rows = if let Some(plan) = implementation.as_ref() {
            self.checklist_rows(&scope.work_sessions, &scope.delegated, plan)
        } else {
            refresh_task_rows_from_sessions(
                &scope.work_sessions,
                &selected.session_id,
                &scope.delegated,
                existing_rows,
                &self.agent_config,
            )
        };
        let error = if rows.len() > scope.work_sessions.len() {
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

    fn store_progress_snapshot(
        &mut self,
        session_id: &str,
        snapshot: CachedProgress,
        resolve_initial_selection: bool,
    ) {
        self.progress_cache
            .insert(session_id.to_string(), snapshot.clone());
        if self.selected_session_id() != Some(session_id) {
            return;
        }
        if self
            .initial_progress_selection
            .as_ref()
            .is_some_and(|selection| selection.parent_session_id != session_id)
        {
            self.initial_progress_selection = None;
        }
        if self.progress_session_id.as_deref() != Some(session_id) {
            self.progress_implementation_expanded = true;
            self.progress_batch_expanded.clear();
        }
        let initial_target = self.initial_progress_target(session_id).map(str::to_owned);
        if let Some(delegate_id) = initial_target.as_deref() {
            self.progress_implementation_expanded = true;
            if let Some(plan) = snapshot.implementation.as_ref() {
                expand_initial_target_batch(
                    plan,
                    Some(delegate_id),
                    &mut self.progress_batch_expanded,
                );
            }
        }
        let initial_selection_index =
            self.initial_progress_row_target(session_id)
                .and_then(|delegate_id| {
                    snapshot
                        .rows
                        .iter()
                        .position(|row| row.selection_key() == Some(delegate_id))
                });
        let (selected_index, selected_key) = if let Some(index) = initial_selection_index {
            let key = snapshot.rows[index].selection_key().map(str::to_owned);
            (index, key)
        } else if self.progress_session_id.as_deref() == Some(session_id) {
            self.current_progress_selection()
        } else {
            snapshot
                .rows
                .iter()
                .position(|row| row.selection_key() == Some(session_id))
                .map(|index| (index, Some(session_id.to_string())))
                .unwrap_or((0, None))
        };
        self.resolve_initial_progress_selection(
            session_id,
            snapshot.implementation.is_some() || resolve_initial_selection,
        );
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
        self.progress_table_state
            .select(table_selection_index(&self.progress_rows, next_index));
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
        let scope = self.progress_scope(&selected);
        let rows = self.checklist_rows(&scope.work_sessions, &scope.delegated, &plan);
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
        work_sessions: &[Session],
        delegated: &[Session],
        plan: &ImplementationPlan,
    ) -> Vec<ProgressRow> {
        let Some(selected) = work_sessions.first() else {
            return Vec::new();
        };
        let use_current_expansion = self.progress_session_id.as_deref()
            == Some(selected.session_id.as_str())
            && self.selected_session_id() == Some(selected.session_id.as_str());
        let mut expanded_batches = if use_current_expansion {
            self.progress_batch_expanded.clone()
        } else {
            HashMap::new()
        };
        let initial_target = self.initial_progress_target(&selected.session_id);
        expand_initial_target_batch(plan, initial_target, &mut expanded_batches);
        build_checklist_progress_rows(
            work_sessions,
            delegated,
            plan,
            initial_target.is_some()
                || !use_current_expansion
                || self.progress_implementation_expanded,
            &expanded_batches,
            &self.agent_config,
        )
    }

    fn initial_progress_target(&self, parent_session_id: &str) -> Option<&str> {
        self.initial_progress_selection
            .as_ref()
            .filter(|selection| {
                selection.parent_session_id == parent_session_id && selection.expand_batch
            })
            .map(|selection| selection.delegate_session_id.as_str())
    }

    fn initial_progress_row_target(&self, parent_session_id: &str) -> Option<&str> {
        self.initial_progress_selection
            .as_ref()
            .filter(|selection| {
                selection.parent_session_id == parent_session_id && selection.select_row
            })
            .map(|selection| selection.delegate_session_id.as_str())
    }

    fn resolve_initial_progress_selection(&mut self, parent_session_id: &str, resolve_batch: bool) {
        let Some(selection) = self
            .initial_progress_selection
            .as_mut()
            .filter(|selection| selection.parent_session_id == parent_session_id)
        else {
            return;
        };
        selection.select_row = false;
        if resolve_batch {
            selection.expand_batch = false;
        }
        if !selection.select_row && !selection.expand_batch {
            self.initial_progress_selection = None;
        }
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
            .select(table_selection_index(&self.progress_rows, selected_index));
    }
}

fn expand_initial_target_batch(
    plan: &ImplementationPlan,
    target: Option<&str>,
    expanded_batches: &mut HashMap<String, bool>,
) {
    if let Some(batch_id) = target.and_then(|delegate_id| plan.batch_id_for_delegate(delegate_id)) {
        expanded_batches.insert(batch_id.to_string(), true);
    }
}

enum FoldTarget {
    Implementation,
    Batch(String),
}
