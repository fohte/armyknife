use std::collections::HashSet;

use super::{App, View};
use crate::commands::agent::tui::tq_sidebar::{
    SidebarRowKind, SidebarSelection, build_sidebar_rows, selection_exists,
};
use crate::commands::agent::tui::tq_snapshot::TqSnapshot;

impl App {
    pub fn set_tq_snapshot(&mut self, mut snapshot: TqSnapshot) {
        let old_position = self.list_state.selected();
        let old_session_id = self
            .selected_session()
            .map(|session| session.session_id.clone());
        let session_ids = self
            .sessions
            .iter()
            .map(|session| session.session_id.as_str())
            .collect::<HashSet<_>>();
        snapshot
            .session_tasks
            .retain(|session_id, _| session_ids.contains(session_id.as_str()));
        let mut visible_task_ids = snapshot
            .session_tasks
            .values()
            .flat_map(|tasks| tasks.iter().map(|task| task.task_id.clone()))
            .collect::<HashSet<_>>();
        let mut pending = visible_task_ids.iter().cloned().collect::<Vec<_>>();
        while let Some(task_id) = pending.pop() {
            if let Some(parent_id) = snapshot
                .tasks
                .get(&task_id)
                .and_then(|task| task.parent_id.as_ref())
                && visible_task_ids.insert(parent_id.clone())
            {
                pending.push(parent_id.clone());
            }
        }
        snapshot
            .tasks
            .retain(|task_id, _| visible_task_ids.contains(task_id));
        let project_ids = snapshot
            .tasks
            .values()
            .filter_map(|task| task.project_id.clone())
            .collect::<HashSet<_>>();
        snapshot
            .projects
            .retain(|project_id, _| project_ids.contains(project_id));
        self.task_by_session = snapshot
            .session_tasks
            .iter()
            .filter_map(|(session_id, tasks)| {
                tasks
                    .first()
                    .cloned()
                    .map(|task| (session_id.clone(), task))
            })
            .collect();
        self.tq_snapshot = Some(snapshot);

        if !selection_exists(self.tq_snapshot.as_ref(), &self.sidebar_selection) {
            self.sidebar_selection = SidebarSelection::All;
        }
        if !selection_exists(self.tq_snapshot.as_ref(), &self.sidebar_cursor) {
            self.sidebar_cursor = self.sidebar_selection.clone();
        }

        self.apply_filter();
        self.restore_selection(old_position, old_session_id.as_deref());
    }

    pub fn tq_refresh_started(&mut self) {
        self.tq_refreshing = true;
        self.tq_refresh_failed = false;
    }

    pub fn tq_refresh_finished(&mut self) {
        self.tq_refreshing = false;
        self.tq_refresh_failed = false;
    }

    pub fn tq_refresh_failed(&mut self) {
        self.tq_refreshing = false;
        self.tq_refresh_failed = true;
    }

    pub fn sidebar_rows(&self) -> Vec<super::super::tq_sidebar::SidebarRow> {
        build_sidebar_rows(
            self.tq_snapshot.as_ref(),
            &self.sessions,
            &self.sidebar_collapsed,
        )
    }

    pub fn set_sidebar_available(&mut self, available: bool) {
        let narrow_layout = self.view == View::Session && !available;
        if self.narrow_layout && !narrow_layout && !self.sidebar_visible {
            self.sidebar_focused = false;
        }
        if available && !self.sidebar_available {
            self.sidebar_focused = false;
        }
        self.narrow_layout = narrow_layout;
        self.sidebar_available = self.view == View::Session && (available || narrow_layout);
        if !self.sidebar_available {
            self.sidebar_focused = false;
        }
    }

    pub fn toggle_sidebar_visibility(&mut self) {
        if self.sidebar_visible {
            self.sidebar_visible = false;
            self.sidebar_focused = false;
        } else if self.sidebar_available {
            self.sidebar_visible = true;
        }
    }

    pub fn toggle_sidebar_focus(&mut self) {
        if self.sidebar_visible && self.sidebar_available {
            self.sidebar_focused = !self.sidebar_focused;
        }
    }

    pub fn show_narrow_sidebar_screen(&mut self) {
        self.sidebar_focused = true;
    }

    pub fn show_narrow_session_list_screen(&mut self) {
        self.sidebar_focused = false;
    }

    pub fn move_sidebar_cursor(&mut self, delta: isize) {
        let rows = self.sidebar_rows();
        let selectable = rows
            .iter()
            .filter_map(|row| row.selection.as_ref())
            .collect::<Vec<_>>();
        if selectable.is_empty() {
            return;
        }
        let current = selectable
            .iter()
            .position(|selection| *selection == &self.sidebar_cursor)
            .unwrap_or(0);
        let next = current
            .saturating_add_signed(delta)
            .min(selectable.len() - 1);
        self.sidebar_cursor = (*selectable[next]).clone();
    }

    pub fn move_sidebar_to_child(&mut self) {
        let rows = self.sidebar_rows();
        let Some(index) = rows
            .iter()
            .position(|row| row.selection.as_ref() == Some(&self.sidebar_cursor))
        else {
            return;
        };
        let row = &rows[index];
        if row.kind == SidebarRowKind::Separator || !row.has_children {
            return;
        }
        if !row.expanded {
            self.sidebar_collapsed.remove(&self.sidebar_cursor);
            return;
        }
        if let Some(child) = rows
            .iter()
            .skip(index + 1)
            .take_while(|child| child.depth > row.depth)
            .find_map(|child| child.selection.clone())
        {
            self.sidebar_cursor = child;
        }
    }

    pub fn move_sidebar_to_parent(&mut self) {
        let rows = self.sidebar_rows();
        let Some(index) = rows
            .iter()
            .position(|row| row.selection.as_ref() == Some(&self.sidebar_cursor))
        else {
            return;
        };
        let row = &rows[index];
        if row.has_children && row.expanded {
            self.sidebar_collapsed.insert(self.sidebar_cursor.clone());
            return;
        }
        if row.depth == 0 {
            return;
        }
        if let Some(parent) = rows[..index]
            .iter()
            .rev()
            .find(|candidate| candidate.depth < row.depth && candidate.selection.is_some())
            .and_then(|candidate| candidate.selection.clone())
        {
            self.sidebar_cursor = parent;
        }
    }

    pub fn select_sidebar_cursor(&mut self) {
        let rows = self.sidebar_rows();
        if !rows
            .iter()
            .any(|row| row.selection.as_ref() == Some(&self.sidebar_cursor))
        {
            return;
        }
        if self.sidebar_selection == self.sidebar_cursor {
            return;
        }
        let old_position = self.list_state.selected();
        let old_session_id = self
            .selected_session()
            .map(|session| session.session_id.clone());
        self.sidebar_selection = self.sidebar_cursor.clone();
        self.apply_filter();
        self.restore_selection(old_position, old_session_id.as_deref());
    }

    pub fn sync_sidebar_list_state(&mut self, rows: &[super::super::tq_sidebar::SidebarRow]) {
        let selection = if self.sidebar_focused {
            &self.sidebar_cursor
        } else {
            &self.sidebar_selection
        };
        let selected = rows
            .iter()
            .position(|row| row.selection.as_ref() == Some(selection));
        self.sidebar_list_state.select(selected);
    }
}
