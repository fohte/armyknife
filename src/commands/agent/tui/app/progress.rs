use std::collections::HashMap;

use chrono::{DateTime, Utc};

use super::super::progress::{
    FetchedProgress, ProgressFetchScope, ProgressRow, ProgressSelectionKey,
    build_task_progress_rows, refresh_task_rows_from_sessions, table_selection_index,
};
use super::super::progress_batches::{
    ImplementationPlan, attach_session_workers,
    build_task_progress_rows as build_checklist_progress_rows,
};
use super::App;
use super::progress_cache::{expand_initial_target_batch, same_implementation_scope};
use crate::commands::agent::types::Session;

mod folding;

#[derive(Clone)]
pub(super) struct CachedProgress {
    pub(super) rows: Vec<ProgressRow>,
    pub(super) error: Option<String>,
    pub(super) implementation: Option<ImplementationPlan>,
    pub(super) request_scope: Option<ProgressFetchScope>,
    pub(super) delegated_sessions: Vec<Session>,
    pub(super) fetched_at: Option<DateTime<Utc>>,
    pub(super) last_attempt_at: Option<DateTime<Utc>>,
}

impl App {
    pub(in crate::commands::agent::tui) fn begin_progress_refresh(&mut self, session_id: &str) {
        if let Some(snapshot) = self.cached_progress(session_id).cloned() {
            let rows = self.refresh_cached_work_rows(session_id, &snapshot.rows);
            let mut snapshot = snapshot;
            snapshot.rows = self.rows_with_current_panes(&rows);
            self.store_progress_snapshot(session_id, snapshot, false);
        } else {
            self.refresh_local_progress(session_id, None, false);
        }
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
                let request_scope = scope.kind.fetch_scope();
                if progress
                    .request_scope
                    .as_ref()
                    .is_some_and(|scope| request_scope.as_ref() != Some(scope))
                {
                    return;
                }
                let includes_fetched_progress = scope.kind.includes_fetched_progress();
                let reusable_cache = self.cached_progress(session_id).filter(|cached| {
                    same_implementation_scope(cached.request_scope.as_ref(), request_scope.as_ref())
                });
                let cached_implementation =
                    reusable_cache.and_then(|cached| cached.implementation.clone());
                let implementation = includes_fetched_progress
                    .then(|| progress.implementation.or(cached_implementation))
                    .flatten();
                let mut delegated = if includes_fetched_progress {
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
                for session in &scope.children {
                    if let Some(fetched) = delegated
                        .iter_mut()
                        .find(|fetched| fetched.session.session_id == session.session_id)
                    {
                        fetched.session = session.clone();
                    } else {
                        delegated.push(super::super::progress::FetchedDelegate {
                            session: session.clone(),
                            checklist_item_id: None,
                            pull_request: None,
                            lookup_unavailable: false,
                        });
                    }
                }
                if let Some(cached) = reusable_cache {
                    delegated.sort_by_key(|delegate| {
                        cached
                            .delegated_sessions
                            .iter()
                            .position(|session| session.session_id == delegate.session.session_id)
                            .unwrap_or(usize::MAX)
                    });
                }
                let has_delegates = !delegated.is_empty();
                let rows = if let Some(plan) = implementation.as_ref() {
                    self.checklist_rows(
                        session_id,
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
                let fetched_at = Utc::now();
                self.progress_fetch_states.insert(
                    session_id.to_string(),
                    super::super::progress::ProgressFetchState::Ready,
                );
                self.store_progress_snapshot(
                    session_id,
                    CachedProgress {
                        rows,
                        error,
                        implementation,
                        request_scope,
                        delegated_sessions: delegated
                            .iter()
                            .map(|delegate| delegate.session.clone())
                            .collect(),
                        fetched_at: Some(fetched_at),
                        last_attempt_at: Some(fetched_at),
                    },
                    true,
                );
            }
            Ok(_) => {}
            Err(error) => {
                self.progress_fetch_states.insert(
                    session_id.to_string(),
                    super::super::progress::ProgressFetchState::Failed(error.clone()),
                );
                self.refresh_local_progress(session_id, Some(error), true);
            }
        }
    }

    fn existing_progress_rows(&self, session_id: &str) -> Vec<ProgressRow> {
        self.cached_progress(session_id)
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
        let current_request_scope = scope.kind.fetch_scope();
        let cached = self.cached_progress(&selected.session_id);
        let reusable_cache = cached.filter(|snapshot| {
            same_implementation_scope(
                snapshot.request_scope.as_ref(),
                current_request_scope.as_ref(),
            )
        });
        let implementation = reusable_cache.and_then(|snapshot| snapshot.implementation.clone());
        let cached_delegates = reusable_cache
            .map(|snapshot| snapshot.delegated_sessions.as_slice())
            .unwrap_or_default();
        let delegated =
            self.delegates_with_missing_cached_sessions(&scope.children, cached_delegates);
        let rows = if let Some(plan) = implementation.as_ref() {
            self.checklist_rows(&selected.session_id, &scope.work_sessions, &delegated, plan)
        } else {
            refresh_task_rows_from_sessions(
                &scope.work_sessions,
                &selected.session_id,
                &scope.children,
                existing_rows,
                &self.agent_config,
            )
        };
        let error = if rows.len() > scope.work_sessions.len() {
            cached.and_then(|snapshot| snapshot.error.clone())
        } else {
            None
        };
        CachedProgress {
            rows,
            error,
            implementation,
            request_scope: cached
                .map(|snapshot| snapshot.request_scope.clone())
                .unwrap_or(current_request_scope),
            delegated_sessions: delegated,
            fetched_at: cached.and_then(|snapshot| snapshot.fetched_at),
            last_attempt_at: cached.and_then(|snapshot| snapshot.last_attempt_at),
        }
    }

    fn delegates_with_missing_cached_sessions(
        &self,
        current: &[Session],
        cached: &[Session],
    ) -> Vec<Session> {
        let mut delegates = Vec::with_capacity(current.len().max(cached.len()));
        for cached_session in cached {
            if let Some(session) = current
                .iter()
                .find(|session| session.session_id == cached_session.session_id)
            {
                delegates.push(session.clone());
            } else if self.session_by_id(&cached_session.session_id).is_none() {
                delegates.push(cached_session.clone());
            }
        }
        for session in current {
            if !delegates
                .iter()
                .any(|delegate| delegate.session_id == session.session_id)
            {
                delegates.push(session.clone());
            }
        }
        delegates
    }

    fn store_progress_snapshot(
        &mut self,
        session_id: &str,
        snapshot: CachedProgress,
        resolve_initial_selection: bool,
    ) {
        let is_open_source = self.progress_inline_expanded
            && self.progress_inline_source_id.as_deref() == Some(session_id);
        let is_active_source = if self.progress_inline_expanded {
            is_open_source
        } else {
            self.selected_progress_source_id().as_deref() == Some(session_id)
        };
        let inline_selection = is_open_source.then(|| self.selection_anchor());
        self.set_cached_progress(session_id, snapshot.clone());
        if !is_active_source {
            return;
        }
        let rows_unchanged = self.progress_session_id.as_deref() == Some(session_id)
            && self.progress_rows == snapshot.rows;
        if self
            .initial_progress_selection
            .as_ref()
            .is_some_and(|selection| selection.parent_session_id != session_id)
        {
            self.initial_progress_selection = None;
        }
        if self.progress_session_id.as_deref() != Some(session_id) {
            self.progress_implementation_expanded = true;
            self.progress_unmatched_expanded = true;
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
                    snapshot.rows.iter().position(|row| {
                        row.selection_key()
                            == Some(ProgressSelectionKey::Session(delegate_id.to_string()))
                    })
                });
        let (selected_index, selected_key) = if let Some(index) = initial_selection_index {
            let key = snapshot.rows[index].selection_key();
            (index, key)
        } else if self.progress_session_id.as_deref() == Some(session_id) {
            self.current_progress_selection()
        } else {
            snapshot
                .rows
                .iter()
                .position(|row| {
                    row.selection_key()
                        == Some(ProgressSelectionKey::Session(session_id.to_string()))
                })
                .map(|index| {
                    (
                        index,
                        Some(ProgressSelectionKey::Session(session_id.to_string())),
                    )
                })
                .unwrap_or((0, None))
        };
        self.resolve_initial_progress_selection(
            session_id,
            snapshot.implementation.is_some() || resolve_initial_selection,
        );
        self.progress_session_id = Some(session_id.to_string());
        self.progress_error = snapshot.error.clone();
        if !rows_unchanged {
            self.replace_progress_rows(snapshot.rows, selected_key, selected_index);
            if let Some(selection) = inline_selection {
                self.restore_selection(selection);
            }
        }
    }

    fn checklist_rows(
        &self,
        source_session_id: &str,
        work_sessions: &[Session],
        delegated: &[Session],
        plan: &ImplementationPlan,
    ) -> Vec<ProgressRow> {
        let Some(selected) = self.session_by_id(source_session_id) else {
            return Vec::new();
        };
        let plan = attach_session_workers(plan, work_sessions, delegated);
        let use_current_expansion =
            self.progress_session_id.as_deref() == Some(selected.session_id.as_str());
        let mut expanded_batches = if use_current_expansion {
            self.progress_batch_expanded.clone()
        } else {
            HashMap::new()
        };
        let initial_target = self.initial_progress_target(source_session_id);
        expand_initial_target_batch(&plan, initial_target, &mut expanded_batches);
        let unmatched_expanded = initial_target
            .is_some_and(|target| plan.batch_id_for_worker(target).is_none())
            || !use_current_expansion
            || self.progress_unmatched_expanded;
        build_checklist_progress_rows(
            work_sessions,
            delegated,
            &plan,
            initial_target.is_some()
                || !use_current_expansion
                || self.progress_implementation_expanded,
            unmatched_expanded,
            &expanded_batches,
            &self.agent_config,
        )
    }

    fn refresh_cached_work_rows(
        &self,
        session_id: &str,
        cached_rows: &[ProgressRow],
    ) -> Vec<ProgressRow> {
        let Some(selected) = self.session_by_id(session_id) else {
            return cached_rows.to_vec();
        };
        let scope = self.progress_scope(&selected);
        let request_scope = scope.kind.fetch_scope();
        let cached = self.cached_progress(session_id);
        let reusable_cache = cached.filter(|snapshot| {
            same_implementation_scope(snapshot.request_scope.as_ref(), request_scope.as_ref())
        });
        if let Some(plan) = reusable_cache.and_then(|snapshot| snapshot.implementation.as_ref()) {
            let cached_delegates = reusable_cache
                .map(|snapshot| snapshot.delegated_sessions.as_slice())
                .unwrap_or_default();
            let delegated =
                self.delegates_with_missing_cached_sessions(&scope.children, cached_delegates);
            return self.checklist_rows(session_id, &scope.work_sessions, &delegated, plan);
        }
        let refreshed_rows = refresh_task_rows_from_sessions(
            &scope.work_sessions,
            session_id,
            &scope.children,
            cached_rows,
            &self.agent_config,
        );
        refreshed_rows
            .into_iter()
            .take_while(|row| matches!(row, ProgressRow::Work(_)))
            .chain(
                cached_rows
                    .iter()
                    .filter(|row| !matches!(row, ProgressRow::Work(_)))
                    .cloned(),
            )
            .collect()
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
            .and_then(|session_id| self.cached_progress(session_id))
            .and_then(|snapshot| snapshot.implementation.as_ref())
    }

    fn replace_progress_rows(
        &mut self,
        rows: Vec<ProgressRow>,
        selected_key: Option<ProgressSelectionKey>,
        previous_index: usize,
    ) {
        let selected_index = selected_key
            .and_then(|key| {
                rows.iter()
                    .position(|row| row.selection_key() == Some(key.clone()))
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
        if let Some(session_id) = self.progress_session_id.clone() {
            let rows = self.progress_rows.clone();
            if let Some(snapshot) = self.cached_progress_mut(&session_id) {
                snapshot.rows = rows;
            }
        }
    }
}
