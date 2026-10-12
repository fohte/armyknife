use crate::commands::agent::types::{Session, SessionStatus};
use chrono::{DateTime, Utc};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::ListItem,
};
use unicode_width::UnicodeWidthStr;

use crate::commands::agent::tui::app::App;
use crate::commands::agent::tui::session_rows::{
    Section, SectionHeaderRow, SessionRowEntry, TaskMark, is_related_task, kin_relation,
};
use crate::commands::agent::user_turn::{UserTurn, UserTurnKind};

use super::super::helpers::{
    DIM_FG, get_session_info, get_title_display_name_fallback, highlight_matches, kin_color,
    status_color, truncate,
};
use super::task_number::task_number_style;

/// Display width reserved by ratatui's `List::highlight_symbol` (the `>`
/// selection marker). Every row -- selected or not -- occupies this column,
/// so it counts toward the fixed-width column budget below even though it
/// is never part of a `Line`'s own spans.
const MARKER_WIDTH: usize = 1;
/// Status glyph (1 col, `session.display_symbol()` is always single-width)
/// plus one space of padding before the repo column.
const STATUS_COLUMN_WIDTH: usize = 2;
/// Fixed width of the repo column, left-aligned and space-padded.
const REPO_COLUMN_WIDTH: usize = 16;
/// Fixed width of the task number column.
pub(super) const TASK_NUMBER_COLUMN_WIDTH: usize = 6;
/// Minimum width of the work type icon plus its trailing gap. The icon portion
/// grows when a configured icon occupies more than one terminal column.
pub(super) const MIN_WORK_TYPE_COLUMN_WIDTH: usize = 2;
/// Fixed width of the right-aligned time column.
const TIME_COLUMN_WIDTH: usize = 9;
/// The longest automatic user-turn label is `failed`; configured names can be wider.
const USER_TURN_LABEL_WIDTH: usize = 6;
/// Floor for the variable-width title column so it never collapses to
/// nothing on very narrow terminals.
const MIN_TITLE_WIDTH: usize = 10;
/// Prefix width before the work type column, shared by title and
/// waiting-question alignment.
const TITLE_COLUMN_BASE_INDENT: usize =
    MARKER_WIDTH + STATUS_COLUMN_WIDTH + REPO_COLUMN_WIDTH + TASK_NUMBER_COLUMN_WIDTH;
pub(super) const WAITING_QUESTION_BASE_INDENT: usize = TITLE_COLUMN_BASE_INDENT;
/// Below this age, the time column renders in the default (bright)
/// foreground; at or above it, it dims to `DIM_FG`. Independent of status
/// color, so a stale RUNNING session's time still reads as stale.
const RECENT_TIME_THRESHOLD_SECS: i64 = 3600;
/// Width of the work type icon column plus its trailing gap, expanded for the
/// widest configured icon so every row's title starts in the same column.
fn work_type_column_width(app: &App) -> usize {
    app.agent_config
        .work_types
        .values()
        .map(|config| config.icon.width())
        .max()
        .unwrap_or(1)
        .max(MIN_WORK_TYPE_COLUMN_WIDTH - 1)
        + 1
}

/// Variable width left for the title after fixed columns, floored so it never
/// disappears on narrow terminals.
fn title_column_width(term_width: usize, work_type_column_width: usize) -> usize {
    term_width
        .saturating_sub(TITLE_COLUMN_BASE_INDENT + work_type_column_width + TIME_COLUMN_WIDTH)
        .max(MIN_TITLE_WIDTH)
}

/// Pads `s` with trailing spaces up to `width` display columns (no-op if
/// already at or over width). Used for left-aligned fixed-width columns.
fn pad_to_width(s: &str, width: usize) -> String {
    let display_width = s.width();
    if display_width >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - display_width))
    }
}

/// Pads `s` with leading spaces up to `width` display columns (no-op if
/// already at or over width). Used for the right-aligned time column.
fn pad_left_to_width(s: &str, width: usize) -> String {
    let display_width = s.width();
    if display_width >= width {
        s.to_string()
    } else {
        format!("{}{s}", " ".repeat(width - display_width))
    }
}

/// Compact relative-time formatter for the fixed-width time column.
/// Deliberately not `worktree_session_children::format_relative_time`
/// (which returns the longer `"{n}m ago"` style) -- this column is too
/// narrow for that and every row must fit the same 9-column budget.
fn format_compact_time(dt: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let seconds = now.signed_duration_since(dt).num_seconds().max(0);
    if seconds < 60 {
        return "just now".to_string();
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours}h");
    }
    format!("{}d", hours / 24)
}

/// Renders a user-turn row with its session name ahead of the note.
pub(super) fn build_user_turn_item(
    turn: &UserTurn<'_>,
    app: &App,
    now: DateTime<Utc>,
    term_width: usize,
    is_linked: bool,
) -> ListItem<'static> {
    let title = app
        .get_cached_title(&turn.session.session_id)
        .map(String::from)
        .unwrap_or_else(|| get_title_display_name_fallback(turn.session));
    let note = turn
        .note
        .map(|note| note.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|note| !note.is_empty());
    let time_text = format_compact_time(turn.session.updated_at, now);
    let time_col = pad_left_to_width(&time_text, TIME_COLUMN_WIDTH);
    let full_label_width = turn
        .label
        .map(|label| label.width().max(USER_TURN_LABEL_WIDTH));
    let line_width = term_width.saturating_sub(MARKER_WIDTH);
    let full_label_description_width = full_label_width.map_or_else(
        || line_width.saturating_sub(STATUS_COLUMN_WIDTH + TIME_COLUMN_WIDTH),
        |label_width| {
            let label_column_width = label_width + 3;
            line_width.saturating_sub(STATUS_COLUMN_WIDTH + label_column_width + TIME_COLUMN_WIDTH)
        },
    );
    let status_label_hides_note = turn.kind == UserTurnKind::AgentStatus
        && note.as_ref().is_some_and(|note| {
            let required_width = title.width() + 1 + note.width();
            required_width > full_label_description_width
        });
    let label = turn
        .label
        .filter(|_| !status_label_hides_note)
        .map(|label| {
            let label_width = label.width().max(USER_TURN_LABEL_WIDTH);
            let padding = " ".repeat(label_width.saturating_sub(label.width()));
            format!(" {label}{padding} ")
        });
    let label_width = label.as_ref().map_or(0, |label| label.width() + 1);
    let fixed_width = 1 + 1 + label_width + TIME_COLUMN_WIDTH;
    let description_width = line_width.saturating_sub(fixed_width);
    let mut description = truncate(&title, description_width);
    if let Some(note) = note {
        let note_width = description_width.saturating_sub(description.width() + 1);
        if note_width > 0 {
            description.push(' ');
            description.push_str(&truncate(&note, note_width));
        }
    }
    let description_padding = " ".repeat(description_width.saturating_sub(description.width()));
    let symbol_style = Style::default().fg(Color::Red);
    let mut spans = vec![
        Span::styled(turn.kind.symbol(), symbol_style),
        Span::raw(" "),
    ];
    if let Some(label) = label {
        spans.push(Span::styled(label, user_turn_label_style()));
        spans.push(Span::raw(" "));
    }
    spans.extend([
        Span::raw(description),
        Span::raw(description_padding),
        Span::raw(time_col),
    ]);

    let item = ListItem::new(Line::from(spans));
    if is_linked {
        item.style(Style::default().bg(Color::Indexed(236)))
    } else {
        item
    }
}

fn user_turn_label_style() -> Style {
    Style::default()
        .fg(Color::White)
        .bg(Color::Red)
        .add_modifier(Modifier::BOLD)
}

fn header_style(kind: Section) -> Style {
    let base = Style::default().add_modifier(Modifier::BOLD);
    match kind {
        Section::UserTurn => base.fg(Color::Red),
        Section::Tasks => base.fg(DIM_FG),
    }
}

/// `◐` shares YOUR TURN's red so the user's own turns stand out in the tree.
fn mark_glyph(mark: TaskMark, session: &Session) -> (&'static str, Color) {
    match mark {
        TaskMark::Done => ("✓", DIM_FG),
        TaskMark::UserTurn => ("◐", Color::Red),
        TaskMark::Working => ("●", Color::Green),
        TaskMark::Waiting => ("◷", Color::Yellow),
        TaskMark::Unclassified => {
            let status = session.display_status();
            (status.display_symbol(), status_color(status))
        }
    }
}

/// Style for a session's own title text (not the tree branch or
/// badge, which stay `DIM_FG` regardless -- see `build_title_spans`).
///
/// Idleness is expressed only through bold/non-bold here, never through
/// color: `kin_color` (when `Some`) owns the color axis to show the
/// session's kinship to the cursor, so an idle kin row still needs its hue
/// visible rather than washed out by `DIM_FG`. Only a non-kin row falls
/// back to the old `DIM_FG`-when-idle look.
fn own_title_style(is_idle: bool, kin_color: Option<Color>) -> Style {
    let style = if is_idle {
        Style::default()
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    };
    match kin_color {
        Some(color) => style.fg(color),
        None if is_idle => style.fg(DIM_FG),
        None => style,
    }
}

/// Renders a section header as a horizontal rule with the label inline,
/// e.g. `── TASKS ──...──`.
///
/// `with_leading_blank` prepends a blank line as a visual separator from
/// the previous section (every header but the very first one in the list).
/// It lives inside this `ListItem` rather than as a standalone item so that
/// `items` stays index-aligned with `rows` (see the caller).
pub(super) fn build_header_item(
    header: &SectionHeaderRow,
    term_width: usize,
    with_leading_blank: bool,
) -> ListItem<'static> {
    let content_width = term_width.saturating_sub(MARKER_WIDTH);
    let style = header_style(header.kind);
    let prefix = format!("── {} ", header.label);
    let dashes = content_width.saturating_sub(prefix.width());
    let content = format!("{prefix}{}", "─".repeat(dashes));

    let mut lines = Vec::with_capacity(2);
    if with_leading_blank {
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled(content, style)));

    ListItem::new(lines)
}

/// Renders one TASKS row: mark, fixed-width repo column, variable-width
/// title column (with a tree branch on child rows), and a right-aligned
/// fixed-width time column. `WaitingInput` sessions get a second line
/// holding only the question.
pub(super) fn build_session_item(
    entry: &SessionRowEntry,
    app: &App,
    now: DateTime<Utc>,
    term_width: usize,
    query: &str,
    is_linked: bool,
) -> ListItem<'static> {
    let session = entry.session;
    let is_idle = matches!(entry.mark, TaskMark::Done | TaskMark::Unclassified);
    let (symbol, symbol_color) = mark_glyph(entry.mark, session);
    let status_style = Style::default().fg(symbol_color);

    let repo_name = app
        .get_cached_worktree_labels(&session.cwd)
        .map_or("", |(repo, _)| repo);
    let repo_text = get_session_info(session, repo_name);
    let repo_col = pad_to_width(&truncate(&repo_text, REPO_COLUMN_WIDTH), REPO_COLUMN_WIDTH);

    let own_title = match entry.task.as_ref() {
        Some(task) => task.task_title.clone(),
        None => app
            .get_cached_title(&session.session_id)
            .map(String::from)
            .unwrap_or_else(|| get_title_display_name_fallback(session)),
    };

    let title_kin_color = app
        .selected_session()
        .and_then(|selected| kin_relation(selected, session))
        .and_then(|(direction, distance)| kin_color(direction, distance));

    let cursor_task = app
        .selected_session()
        .and_then(|selected| app.task_by_session.get(selected.session_id.as_str()));
    let task_number_spans = if let Some(task) = entry.task.as_ref() {
        let related = is_related_task(cursor_task, Some(task));
        let style = task_number_style(related, task.is_closed);
        let number = truncate(
            &format!("#{}", task.task_number),
            TASK_NUMBER_COLUMN_WIDTH.saturating_sub(1),
        );
        let number_width = number.width();
        let number_style = if task.is_closed {
            style.add_modifier(Modifier::CROSSED_OUT)
        } else {
            style
        };
        vec![
            Span::styled(number, number_style),
            Span::raw(" ".repeat(TASK_NUMBER_COLUMN_WIDTH.saturating_sub(number_width))),
        ]
    } else if app.is_tq_loading() {
        vec![
            Span::styled("━━━━", Style::default().fg(DIM_FG)),
            Span::raw(" ".repeat(TASK_NUMBER_COLUMN_WIDTH - "━━━━".width())),
        ]
    } else {
        vec![Span::raw(" ".repeat(TASK_NUMBER_COLUMN_WIDTH))]
    };

    let work_type_column_width = work_type_column_width(app);
    let work_type_spans = session
        .work_type
        .as_deref()
        .and_then(|skill_name| app.agent_config.work_type(skill_name))
        .map_or_else(
            || vec![Span::raw(" ".repeat(work_type_column_width))],
            |config| {
                let icon_width = config.icon.width();
                vec![
                    Span::styled(
                        config.icon.clone(),
                        Style::default().fg(Color::from(config.color)),
                    ),
                    Span::raw(" ".repeat(work_type_column_width.saturating_sub(icon_width))),
                ]
            },
        );

    let branch = tree_branch_text(entry.depth, entry.is_last_sibling);
    let branch_width = branch.width();
    let title_width =
        title_column_width(term_width, work_type_column_width).saturating_sub(branch_width);
    let title_style = own_title_style(is_idle, title_kin_color);
    let title_spans = build_title_spans(entry, &own_title, title_width, query, title_style);

    let time_text = format_compact_time(session.updated_at, now);
    let time_col = pad_left_to_width(&time_text, TIME_COLUMN_WIDTH);
    let seconds_since_update = now.signed_duration_since(session.updated_at).num_seconds();
    let time_style = if seconds_since_update < RECENT_TIME_THRESHOLD_SECS {
        Style::default()
    } else {
        Style::default().fg(DIM_FG)
    };

    let mut spans = vec![
        Span::styled(symbol, status_style),
        Span::raw(" "),
        Span::styled(repo_col, Style::default().fg(DIM_FG)),
    ];
    spans.extend(task_number_spans);
    spans.push(Span::styled(branch, Style::default().fg(DIM_FG)));
    spans.extend(work_type_spans);
    spans.extend(title_spans);
    spans.push(Span::styled(time_col, time_style));

    let mut lines = vec![Line::from(spans)];

    // An empty question still renders bare quotes rather than an
    // inconsistent row shape. Sessions already in YOUR TURN skip it there.
    if session.status == SessionStatus::WaitingInput && entry.is_selectable {
        let question = session
            .current_tool
            .as_deref()
            .or(session.last_message.as_deref())
            .unwrap_or("");
        let quoted = format!("\u{201c}{question}\u{201d}");
        let question_indent = WAITING_QUESTION_BASE_INDENT + branch_width + work_type_column_width;
        let quoted_width = term_width.saturating_sub(question_indent);
        let truncated_quoted = truncate(&quoted, quoted_width);
        // ratatui reserves the marker column on every line of a multi-line
        // `ListItem`, not just the first, so our own content only needs to
        // cover the columns before the title to reach `question_indent`.
        lines.push(Line::from(vec![
            Span::raw(" ".repeat(question_indent - MARKER_WIDTH)),
            Span::styled(truncated_quoted, Style::default().fg(DIM_FG)),
        ]));
    }

    let item = ListItem::new(lines);
    if is_linked {
        item.style(Style::default().bg(Color::Indexed(236)))
    } else {
        item
    }
}

fn tree_branch_text(depth: usize, is_last_sibling: bool) -> &'static str {
    match (depth, is_last_sibling) {
        (0, _) => "",
        (_, true) => "└─ ",
        (_, false) => "├─ ",
    }
}

/// `◐ {user turns}/{children}` on roots that have children (PM-style rows).
fn child_badge_text(child_count: usize, child_user_turn_count: usize) -> String {
    if child_count == 0 {
        String::new()
    } else {
        format!(" \u{25d0} {}/{}", child_user_turn_count, child_count)
    }
}

/// The badge's width is carved out of `title_width` up front so the title
/// truncates to leave room for it, and the badge is appended right after
/// that (unpadded) content -- rather than after padding, which would push it
/// flush against the time column instead of next to the title it describes.
fn build_title_spans(
    entry: &SessionRowEntry,
    own_title: &str,
    title_width: usize,
    query: &str,
    title_style: Style,
) -> Vec<Span<'static>> {
    let child_badge = child_badge_text(entry.child_count, entry.child_user_turn_count);
    let crit_badge = if entry.session.crit_urls.is_empty() {
        String::new()
    } else {
        " [crit]".to_string()
    };
    let show_child_badge =
        !child_badge.is_empty() && child_badge.width() + crit_badge.width() < title_width;
    let child_badge = if show_child_badge {
        child_badge
    } else {
        String::new()
    };
    let badge_width = child_badge.width() + crit_badge.width();
    let content_width = title_width.saturating_sub(badge_width);

    let title = truncate(own_title, content_width);
    let mut used_width = title.width();
    let mut spans = highlight_matches(&title, query, title_style);
    if !child_badge.is_empty() {
        used_width += child_badge.width();
        spans.push(Span::styled(child_badge, Style::default().fg(DIM_FG)));
    }
    if !crit_badge.is_empty() {
        used_width += crit_badge.width();
        spans.push(Span::styled(
            crit_badge,
            Style::default().fg(Color::Indexed(98)),
        ));
    }
    if used_width < title_width {
        spans.push(Span::raw(" ".repeat(title_width - used_width)));
    }

    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    #[rstest]
    #[case::just_now(0, "just now")]
    #[case::one_minute(60, "1m")]
    #[case::two_hours(7200, "2h")]
    #[case::one_day(86400, "1d")]
    fn test_format_compact_time(#[case] seconds_ago: i64, #[case] expected: &str) {
        let now = Utc::now();
        let dt = now - chrono::Duration::seconds(seconds_ago);
        assert_eq!(format_compact_time(dt, now), expected);
    }

    #[rstest]
    #[case::user_turn(
        Section::UserTurn,
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    )]
    #[case::tasks(
        Section::Tasks,
        Style::default().fg(DIM_FG).add_modifier(Modifier::BOLD)
    )]
    fn test_header_style(#[case] kind: Section, #[case] expected: Style) {
        assert_eq!(header_style(kind), expected);
    }

    #[test]
    fn test_user_turn_label_has_a_red_background() {
        assert_eq!(
            user_turn_label_style(),
            Style::default()
                .fg(Color::White)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD)
        );
    }

    #[rstest]
    #[case::idle_no_kin(true, None, Style::default().fg(DIM_FG))]
    #[case::active_no_kin(false, None, Style::default().add_modifier(Modifier::BOLD))]
    #[case::idle_kin(
        true,
        Some(Color::Indexed(206)),
        Style::default().fg(Color::Indexed(206))
    )]
    #[case::active_kin(
        false,
        Some(Color::Indexed(39)),
        Style::default().add_modifier(Modifier::BOLD).fg(Color::Indexed(39))
    )]
    fn test_own_title_style(
        #[case] is_idle: bool,
        #[case] kin_color: Option<Color>,
        #[case] expected: Style,
    ) {
        assert_eq!(own_title_style(is_idle, kin_color), expected);
    }

    #[rstest]
    #[case::done(TaskMark::Done, ("✓", DIM_FG))]
    #[case::user_turn(TaskMark::UserTurn, ("◐", Color::Red))]
    #[case::working(TaskMark::Working, ("●", Color::Green))]
    #[case::waiting(TaskMark::Waiting, ("◷", Color::Yellow))]
    #[case::unclassified_uses_status_glyph(TaskMark::Unclassified, ("⏸", DIM_FG))]
    fn test_mark_glyph(#[case] mark: TaskMark, #[case] expected: (&str, Color)) {
        let mut session = crate::commands::agent::tui::ui::test_support::create_test_session("s");
        session.status = SessionStatus::Paused;
        assert_eq!(mark_glyph(mark, &session), expected);
    }

    #[rstest]
    #[case::no_children_shows_no_badge(0, 0, "")]
    #[case::children_without_user_turns(3, 0, " ◐ 0/3")]
    #[case::children_with_user_turns(10, 3, " ◐ 3/10")]
    fn test_child_badge_text(
        #[case] child_count: usize,
        #[case] child_user_turn_count: usize,
        #[case] expected: &str,
    ) {
        assert_eq!(
            child_badge_text(child_count, child_user_turn_count),
            expected
        );
    }

    #[rstest]
    #[case::root(0, true, "")]
    #[case::middle_child(1, false, "├─ ")]
    #[case::last_child(1, true, "└─ ")]
    fn test_tree_branch_text(
        #[case] depth: usize,
        #[case] is_last_sibling: bool,
        #[case] expected: &str,
    ) {
        assert_eq!(tree_branch_text(depth, is_last_sibling), expected);
    }
}
