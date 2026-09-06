use ratatui::style::{Color, Style};

use crate::commands::cc::tui::ui::helpers::DIM_FG;

/// Text and styling for the `#<number> <title> › ` task-prefix, split into
/// the strikethrough-eligible label (`#<number> <title>`) and the trailing
/// ` › ` separator, which never gets struck through: the tq task being
/// closed says nothing about the session or its title after the separator
/// (see `build_breadcrumb_title_spans` in the parent module).
pub(super) struct TaskPrefixSpan {
    pub(super) text: String,
    pub(super) style: Style,
    pub(super) label_style: Style,
    pub(super) label_len: usize,
}

/// Task title-prefix style: dusty purple ([`Color::Indexed(97)`]) whenever
/// the linked tq task is closed -- this overrides the cursor-relatedness
/// dimming entirely, so a closed task reads as closed regardless of which
/// row is selected. Otherwise plain/default when the row's task is related
/// to the cursor row's task (see `is_related_task` in the parent module),
/// `DIM_FG` otherwise. A channel separate from `own_title_style`'s
/// `kin_color` -- session kinship colors the title, task
/// kinship/closedness only ever colors this prefix.
pub(super) fn task_prefix_style(is_related: bool, is_closed: bool) -> Style {
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
    fn test_task_prefix_style(
        #[case] is_related: bool,
        #[case] is_closed: bool,
        #[case] expected: Style,
    ) {
        assert_eq!(task_prefix_style(is_related, is_closed), expected);
    }
}
