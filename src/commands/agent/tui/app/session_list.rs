use std::collections::HashSet;

use super::super::progress::ProgressRow;
use super::super::session_rows::inline_progress::project_inline_progress_rows;
use super::super::session_rows::{SessionRow, SessionRowKey, build_session_rows_with_order};
use super::App;

impl App {
    pub(in crate::commands::agent::tui) fn build_display_rows(&self) -> Vec<SessionRow<'_>> {
        let filtered = self.filtered_sessions();
        let rows = build_session_rows_with_order(
            &filtered,
            &self.task_by_session,
            &self.agent_config,
            &self.vibe_enabled_session_ids,
            self.session_row_order.as_ref(),
        );
        let Some(source_session_id) = self.progress_inline_source_id.as_deref() else {
            return rows;
        };
        let progress_rows = if self.progress_session_id.as_deref() == Some(source_session_id) {
            self.progress_rows.as_slice()
        } else {
            &[]
        };
        let matched_sessions = progress_rows
            .iter()
            .filter_map(|row| match row {
                ProgressRow::ChecklistPullRequest { row, .. }
                    if filtered
                        .iter()
                        .any(|session| session.session_id == row.session_id) =>
                {
                    Some(row.session_id.clone())
                }
                _ => None,
            })
            .collect::<HashSet<_>>();
        let moved_sessions = filtered
            .iter()
            .filter(|session| {
                matched_sessions.contains(&session.session_id)
                    || session
                        .ancestor_session_ids
                        .iter()
                        .any(|ancestor| matched_sessions.contains(ancestor))
            })
            .map(|session| session.session_id.as_str())
            .collect::<HashSet<_>>();
        let remaining = filtered
            .iter()
            .copied()
            .filter(|session| !moved_sessions.contains(session.session_id.as_str()))
            .collect::<Vec<_>>();
        let remaining_rows = build_session_rows_with_order(
            &remaining,
            &self.task_by_session,
            &self.agent_config,
            &self.vibe_enabled_session_ids,
            self.session_row_order.as_ref(),
        );
        let status = self.progress_status_label(source_session_id, progress_rows);
        project_inline_progress_rows(
            rows,
            remaining_rows,
            source_session_id,
            progress_rows,
            &matched_sessions,
            status,
        )
    }

    #[cfg(test)]
    pub(in crate::commands::agent::tui) fn update_row_order(
        &mut self,
        row_session_ids: &[Option<&str>],
    ) {
        let row_keys = row_session_ids
            .iter()
            .map(|id| id.map(|id| SessionRowKey::Task(id.to_string())))
            .collect::<Vec<_>>();
        self.update_rendered_row_keys(&row_keys);
    }

    pub(in crate::commands::agent::tui) fn update_rendered_rows(
        &mut self,
        row_keys: &[Option<SessionRowKey>],
        row_heights: &[usize],
        viewport_height: usize,
    ) {
        let selection = self.selection_anchor();
        self.update_rendered_row_keys(row_keys);
        self.row_heights = row_heights.to_vec();
        self.list_viewport_height = viewport_height;
        self.resync_selection(selection);
        self.restore_pending_selection_line();
    }

    pub(super) fn update_rendered_row_keys(&mut self, row_keys: &[Option<SessionRowKey>]) {
        self.row_sessions = row_keys
            .iter()
            .map(|key| {
                key.as_ref().and_then(|key| {
                    let session_id = key.session_id()?;
                    self.sessions
                        .iter()
                        .position(|session| session.session_id == session_id)
                })
            })
            .collect();
        self.row_keys = row_keys.to_vec();
        if let Some(order) = self.session_row_order.as_mut() {
            let keys = self.row_keys.iter().flatten().cloned().collect::<Vec<_>>();
            order.remember_keys(&keys);
        }
    }

    fn restore_pending_selection_line(&mut self) {
        let Some(target_line) = self.pending_selection_line.take() else {
            return;
        };
        let Some(selected) = self.list_state.selected() else {
            return;
        };
        let Some(&selected_height) = self.row_heights.get(selected) else {
            return;
        };
        if self.list_viewport_height == 0 || selected >= self.row_heights.len() {
            return;
        }

        let visible_height = selected_height.min(self.list_viewport_height);
        let max_top = self.list_viewport_height.saturating_sub(visible_height);
        let target_line = target_line.min(max_top);
        let mut best = (selected, usize::MAX);
        for offset in 0..=selected {
            let line = self.row_heights[offset..selected].iter().sum::<usize>();
            if line + visible_height > self.list_viewport_height {
                continue;
            }
            let distance = line.abs_diff(target_line);
            if distance < best.1 {
                best = (offset, distance);
            }
        }
        *self.list_state.offset_mut() = best.0;
    }
}
