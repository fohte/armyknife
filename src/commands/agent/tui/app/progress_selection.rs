use super::super::progress::{ProgressRow, ProgressSelectionKey, table_selection_index};
use super::App;

impl App {
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

    pub(super) fn current_progress_selection(&self) -> (usize, Option<ProgressSelectionKey>) {
        let index = self.progress_list_state.selected().unwrap_or(0);
        let key = self
            .progress_rows
            .get(index)
            .and_then(ProgressRow::selection_key);
        (index, key)
    }
}
