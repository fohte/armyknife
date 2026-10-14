use ratatui::{
    layout::Constraint,
    style::{Color, Style},
};

use crate::commands::agent::tui::progress::ProgressMark;

use super::super::helpers::DIM_FG;

pub(super) fn stage_widths() -> [Constraint; 6] {
    [
        Constraint::Min(8),
        Constraint::Length(4),
        Constraint::Length(4),
        Constraint::Length(5),
        Constraint::Length(3),
        Constraint::Length(6),
    ]
}

pub(super) fn mark_style(mark: ProgressMark) -> Style {
    let color = match mark {
        ProgressMark::Done => DIM_FG,
        ProgressMark::User => Color::Red,
        ProgressMark::Working => Color::Green,
        ProgressMark::Waiting => Color::Yellow,
        ProgressMark::NotStarted => DIM_FG,
    };
    Style::default().fg(color)
}
