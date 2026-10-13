use super::super::progress::{ProgressRow, ProgressSelectionKey, table_selection_index};
use super::App;

#[derive(Clone, PartialEq, Eq)]
pub(super) enum FoldTarget {
    Implementation,
    UnmatchedDelegates,
    Batch(String),
}

impl FoldTarget {
    pub(super) fn for_row(
        row: &ProgressRow,
        action: ProgressFoldAction,
        implementation_expanded: bool,
        unmatched_expanded: bool,
    ) -> Option<(Self, bool)> {
        match row {
            ProgressRow::ImplementationHeader { collapsed, .. } => {
                Some((Self::Implementation, *collapsed))
            }
            ProgressRow::UnmatchedDelegatesHeader { collapsed } => {
                Some((Self::UnmatchedDelegates, *collapsed))
            }
            ProgressRow::BatchHeader {
                batch_id,
                collapsed,
                ..
            } => Some((Self::Batch(batch_id.clone()), *collapsed)),
            ProgressRow::PullRequest(_) if action == ProgressFoldAction::Collapse => {
                Some((Self::UnmatchedDelegates, !unmatched_expanded))
            }
            ProgressRow::Work(_) if action == ProgressFoldAction::Collapse => {
                Some((Self::Implementation, !implementation_expanded))
            }
            ProgressRow::ChecklistPullRequest { batch_id, .. }
                if action == ProgressFoldAction::Collapse =>
            {
                Some((Self::Batch(batch_id.clone()), false))
            }
            _ => None,
        }
    }

    pub(super) fn selection_key(&self) -> ProgressSelectionKey {
        match self {
            Self::Implementation => ProgressSelectionKey::Implementation,
            Self::UnmatchedDelegates => ProgressSelectionKey::UnmatchedDelegates,
            Self::Batch(batch_id) => ProgressSelectionKey::Batch(batch_id.clone()),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ProgressFoldAction {
    Collapse,
    Expand,
    Toggle,
}

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
