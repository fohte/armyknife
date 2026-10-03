use ratatui::style::{Color, Style};

use crate::commands::agent::tui::ui::helpers::DIM_FG;

/// Task-number style: dusty purple ([`Color::Indexed(97)`]) whenever
/// the linked tq task is closed -- this overrides the cursor-relatedness
/// dimming entirely, so a closed task reads as closed regardless of which
/// row is selected. Otherwise plain/default when the row's task is related
/// to the cursor row's task (see `is_related_task` in `session_rows`),
/// `DIM_FG` otherwise. A channel separate from `own_title_style`'s
/// `kin_color` -- session kinship colors the title, task
/// kinship/closedness only ever colors the number.
pub(super) fn task_number_style(is_related: bool, is_closed: bool) -> Style {
    if is_closed {
        Style::default().fg(Color::Indexed(97))
    } else if is_related {
        Style::default()
    } else {
        Style::default().fg(DIM_FG)
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::related_open(true, false, Style::default())]
    #[case::unrelated_open(false, false, Style::default().fg(DIM_FG))]
    #[case::related_closed(true, true, Style::default().fg(Color::Indexed(97)))]
    #[case::unrelated_closed(false, true, Style::default().fg(Color::Indexed(97)))]
    fn test_task_number_style(
        #[case] is_related: bool,
        #[case] is_closed: bool,
        #[case] expected: Style,
    ) {
        assert_eq!(task_number_style(is_related, is_closed), expected);
    }
}
