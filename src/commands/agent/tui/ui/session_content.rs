use chrono::{DateTime, Utc};
use ratatui::{Frame, layout::Rect};

use crate::commands::agent::tui::app::{App, View};

use super::clean_list::render_clean_list;
use super::session_list::render_session_list;

pub(super) fn render_main_list(frame: &mut Frame, area: Rect, app: &mut App, now: DateTime<Utc>) {
    match app.view {
        View::Session => render_session_list(frame, area, app, now),
        View::Clean => render_clean_list(frame, area, app, now),
    }
}
