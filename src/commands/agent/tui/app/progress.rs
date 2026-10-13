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
                let rows = build_progress_rows(&selected, &delegated, &self.agent_config);
                let error = has_delegates.then_some(progress.github_error).flatten();
                self.store_progress_snapshot(session_id, CachedProgress { rows, error });
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
        if snapshot.rows.len() > 1 {
            snapshot.error = error.or(snapshot.error);
        }
        self.store_progress_snapshot(session_id, snapshot);
    }

    fn local_progress_snapshot(
        &self,
        selected: &Session,
        existing_rows: &[ProgressRow],
    ) -> CachedProgress {
        let delegated = direct_delegates(self.sessions.clone(), &selected.session_id);
        let rows =
            refresh_rows_from_sessions(selected, &delegated, existing_rows, &self.agent_config);
        let error = if rows.len() > 1 {
            self.progress_cache
                .get(&selected.session_id)
                .and_then(|snapshot| snapshot.error.clone())
        } else {
            None
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
