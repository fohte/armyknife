use super::super::progress::{FetchedProgress, build_progress_rows, refresh_rows_from_sessions};
use super::super::progress_fetch::direct_delegates;
use super::App;

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
        let selected_row_id = self
            .progress_list_state
            .selected()
            .and_then(|index| self.progress_rows.get(index))
            .map(|row| row.session_id().to_string());
        let delegated = direct_delegates(self.sessions.clone(), session_id);
        self.progress_rows = refresh_rows_from_sessions(
            &selected,
            &delegated,
            &self.progress_rows,
            &self.agent_config,
        );
        let selected_index = selected_row_id
            .and_then(|selected_id| {
                self.progress_rows
                    .iter()
                    .position(|row| row.session_id() == selected_id)
            })
            .unwrap_or(0);
        self.progress_list_state.select(Some(selected_index));
        self.progress_table_state
            .select(selected_index.checked_sub(1));
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
                let selected_progress_session_id = self
                    .progress_list_state
                    .selected()
                    .and_then(|index| self.progress_rows.get(index))
                    .map(|row| row.session_id().to_string());
                self.progress_rows =
                    build_progress_rows(&selected, &progress.delegated, &self.agent_config);
                let selected_index = selected_progress_session_id
                    .and_then(|selected_id| {
                        self.progress_rows
                            .iter()
                            .position(|row| row.session_id() == selected_id)
                    })
                    .unwrap_or(0);
                self.progress_list_state.select(Some(selected_index));
                self.progress_table_state
                    .select(selected_index.checked_sub(1));
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
