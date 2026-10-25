use super::super::progress::{ProgressRow, ProgressSelectionKey};
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
            | ProgressRow::PlannedPullRequest { batch_id, .. }
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
    pub(super) fn current_progress_selection(&self) -> (usize, Option<ProgressSelectionKey>) {
        let index = self.progress_list_state.selected().unwrap_or(0);
        let key = self
            .progress_rows
            .get(index)
            .and_then(ProgressRow::selection_key);
        (index, key)
    }
}
