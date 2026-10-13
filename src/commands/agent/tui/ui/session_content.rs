use chrono::{DateTime, Utc};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
};

use crate::commands::agent::tui::app::{App, View};

use super::clean_list::render_clean_list;
use super::progress::render_progress;
use super::session_list::render_session_list;

const MIN_PANE_WIDTH: u16 = 48;

pub(super) fn minimum_two_pane_width() -> u16 {
    MIN_PANE_WIDTH * 2
}

pub(super) fn render_main_list(frame: &mut Frame, area: Rect, app: &mut App, now: DateTime<Utc>) {
    match app.view {
        View::Session => {
            if app.narrow_layout {
                if app.progress_focused {
                    render_progress(frame, area, app);
                } else {
                    render_session_list(frame, area, app, now);
                }
            } else {
                let [session_area, progress_area] =
                    Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                        .areas(area);
                render_session_list(frame, session_area, app, now);
                render_progress(frame, progress_area, app);
            }
        }
        View::Clean => render_clean_list(frame, area, app, now),
    }
}
