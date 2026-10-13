use super::super::progress::{
    FetchedProgress, ProgressRow, build_progress_rows, refresh_rows_from_sessions,
};
use super::super::progress_fetch::direct_delegates;
use super::App;
use crate::commands::agent::types::Session;

#[derive(Clone)]
pub(super) struct CachedProgress {
    rows: Vec<ProgressRow>,
    error: Option<String>,
}

impl App {
    pub(in crate::commands::agent::tui) fn begin_progress_refresh(
        &mut self,
        session_id: &str,
    ) -> bool {
        let Some(session) = self
            .selected_session()
            .filter(|session| session.session_id == session_id)
            .cloned()
        else {
            return false;
        };
        let rows = self.existing_progress_rows(session_id);
        let snapshot = self.local_progress_snapshot(&session, &rows);
        let should_fetch = snapshot.rows.len() > 1;
        self.store_progress_snapshot(session_id, snapshot);
        should_fetch
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
        let rows = self.existing_progress_rows(session_id);
        let snapshot = self.local_progress_snapshot(&selected, &rows);
        self.store_progress_snapshot(session_id, snapshot);
    }

    pub(in crate::commands::agent::tui) fn apply_progress_result(
        &mut self,
        session_id: &str,
        result: Result<FetchedProgress, String>,
    ) {
        match result {
            Ok(progress) if progress.selected_session_id == session_id => {
                let Some(selected) = self
                    .sessions
                    .iter()
                    .find(|session| session.session_id == session_id)
                    .cloned()
                else {
                    return;
                };
                let fetched_rows =
                    build_progress_rows(&selected, &progress.delegated, &self.agent_config);
                let delegated = direct_delegates(self.sessions.clone(), session_id);
                let rows = refresh_rows_from_sessions(
                    &selected,
                    &delegated,
                    &fetched_rows,
                    &self.agent_config,
                );
                let error = (!delegated.is_empty())
                    .then_some(progress.github_error)
                    .flatten();
                self.store_progress_snapshot(session_id, CachedProgress { rows, error });
            }
            Ok(_) => {}
            Err(error) => {
                let Some(selected) = self
                    .sessions
                    .iter()
                    .find(|session| session.session_id == session_id)
                    .cloned()
                else {
                    return;
                };
                let rows = self.existing_progress_rows(session_id);
                let mut snapshot = self.local_progress_snapshot(&selected, &rows);
                if snapshot.rows.len() > 1 {
                    snapshot.error = Some(error);
                }
                self.store_progress_snapshot(session_id, snapshot);
            }
        }
    }

    fn existing_progress_rows(&self, session_id: &str) -> Vec<ProgressRow> {
        self.progress_cache
            .get(session_id)
            .map(|snapshot| snapshot.rows.clone())
            .or_else(|| {
                (self.progress_session_id.as_deref() == Some(session_id))
                    .then(|| self.progress_rows.clone())
            })
            .unwrap_or_default()
    }

    fn local_progress_snapshot(
        &self,
        selected: &Session,
        existing_rows: &[ProgressRow],
    ) -> CachedProgress {
        let delegated = direct_delegates(self.sessions.clone(), &selected.session_id);
        let rows =
            refresh_rows_from_sessions(selected, &delegated, existing_rows, &self.agent_config);
        let error = if delegated.is_empty() {
            None
        } else {
            self.progress_cache
                .get(&selected.session_id)
                .and_then(|snapshot| snapshot.error.clone())
        };
        CachedProgress { rows, error }
    }

    fn store_progress_snapshot(&mut self, session_id: &str, snapshot: CachedProgress) {
        self.progress_cache
            .insert(session_id.to_string(), snapshot.clone());
        if self.selected_session_id() != Some(session_id) {
            return;
        }
        let selected_row_id = (self.progress_session_id.as_deref() == Some(session_id))
            .then(|| {
                self.progress_list_state
                    .selected()
                    .and_then(|index| self.progress_rows.get(index))
                    .map(|row| row.session_id().to_string())
            })
            .flatten();
        let selected_index = selected_row_id
            .and_then(|selected_id| {
                snapshot
                    .rows
                    .iter()
                    .position(|row| row.session_id() == selected_id)
            })
            .unwrap_or(0);
        self.progress_rows = snapshot.rows;
        self.progress_session_id = Some(session_id.to_string());
        self.progress_error = snapshot.error;
        self.progress_list_state.select(Some(selected_index));
        self.progress_table_state
            .select(selected_index.checked_sub(1));
    }

    pub(in crate::commands::agent::tui) fn select_progress_row(&mut self, delta: isize) {
        if self.progress_rows.is_empty() {
            return;
        }
        let current = self.progress_list_state.selected().unwrap_or(0);
        let next = current
            .saturating_add_signed(delta)
            .min(self.progress_rows.len() - 1);
        self.progress_list_state.select(Some(next));
        self.progress_table_state.select(next.checked_sub(1));
    }
}
