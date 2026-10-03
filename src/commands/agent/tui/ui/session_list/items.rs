use crate::commands::agent::types::SessionStatus;
use chrono::{DateTime, Utc};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::ListItem,
};
use unicode_width::UnicodeWidthStr;

use crate::commands::agent::tui::app::App;
use crate::commands::agent::tui::session_rows::{
    Section, SectionHeaderRow, SessionRowEntry, is_idle_session, is_related_task, kin_relation,
};

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
/// Floor for the variable-width title column so it never collapses to
/// nothing on very narrow terminals.
const MIN_TITLE_WIDTH: usize = 10;
/// Prefix width before the work type column. Added to the configured work
/// type column width to align questions with the title.
pub(super) const WAITING_QUESTION_BASE_INDENT: usize =
    MARKER_WIDTH + STATUS_COLUMN_WIDTH + REPO_COLUMN_WIDTH + TASK_NUMBER_COLUMN_WIDTH;
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
        .saturating_sub(WAITING_QUESTION_BASE_INDENT + work_type_column_width + TIME_COLUMN_WIDTH)
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

/// Section header color: NEEDS YOU and RUNNING echo their status color
/// (amber / green) since they demand attention; UNREAD and the idle
/// (Paused/Stopped) section have no dedicated status color and keep the
/// neutral header look.
fn header_style(kind: Section) -> Style {
    let base = Style::default().add_modifier(Modifier::BOLD);
    match kind {
        Section::NeedsYou => base.fg(Color::Yellow),
        Section::Running => base.fg(Color::Green),
        Section::Unread | Section::Idle => base.fg(DIM_FG),
    }
}

/// Style for a session's own title text (not the breadcrumb prefix or
/// badge, which stay `DIM_FG` regardless -- see `build_breadcrumb_title_spans`).
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
/// e.g. `── RUNNING (3) ──...──`.
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

/// Renders one session row: status glyph, fixed-width repo column,
/// variable-width title column (with breadcrumb prefix when this session
/// has a displayed ancestor), and a right-aligned fixed-width time column.
/// `WaitingInput` sessions get a second line holding only the question.
pub(super) fn build_session_item(
    entry: &SessionRowEntry,
    app: &App,
    now: DateTime<Utc>,
    term_width: usize,
    query: &str,
    is_linked: bool,
) -> ListItem<'static> {
    let session = entry.session;
    let is_idle = is_idle_session(session);

    let display_status = session.display_status();
    let symbol = display_status.display_symbol();
    let status_style = Style::default().fg(status_color(display_status));

    let repo_name = app
        .get_cached_worktree_labels(&session.cwd)
        .map_or("", |(repo, _)| repo);
    let repo_text = get_session_info(session, repo_name);
    let repo_col = pad_to_width(&truncate(&repo_text, REPO_COLUMN_WIDTH), REPO_COLUMN_WIDTH);

    let own_title = app
        .get_cached_title(&session.session_id)
        .map(String::from)
        .unwrap_or_else(|| get_title_display_name_fallback(session));

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

    let title_width = title_column_width(term_width, work_type_column_width);
    let title_style = own_title_style(is_idle, title_kin_color);
    let title_spans = build_title_spans(entry, app, &own_title, title_width, query, title_style);

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
    spans.extend(work_type_spans);
    spans.extend(title_spans);
    spans.push(Span::styled(time_col, time_style));

    let mut lines = vec![Line::from(spans)];

    // The question line is unconditional for every waiting row, so an
    // empty question still renders bare quotes rather than an inconsistent
    // row shape.
    if session.status == SessionStatus::WaitingInput {
        let question = session
            .current_tool
            .as_deref()
            .or(session.last_message.as_deref())
            .unwrap_or("");
        let quoted = format!("\u{201c}{question}\u{201d}");
        let question_indent = WAITING_QUESTION_BASE_INDENT + work_type_column_width;
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

fn descendant_badge_text(descendant_count: usize) -> String {
    if descendant_count == 0 {
        String::new()
    } else {
        format!(" \u{25b8}{descendant_count}")
    }
}

/// The badge's width is carved out of `title_width` up front so the
/// breadcrumb+title portion truncates to leave room for it, and the badge is
/// appended right after that (unpadded) content -- rather than after
/// padding, which would push it flush against the time column instead of
/// next to the title it describes.
fn build_title_spans(
    entry: &SessionRowEntry,
    app: &App,
    own_title: &str,
    title_width: usize,
    query: &str,
    title_style: Style,
) -> Vec<Span<'static>> {
    let dim_style = Style::default().fg(DIM_FG);
    let descendant_badge = descendant_badge_text(entry.descendant_count);
    let crit_badge = if entry.session.crit_urls.is_empty() {
        String::new()
    } else {
        " [crit]".to_string()
    };
    let show_descendant_badge =
        !descendant_badge.is_empty() && descendant_badge.width() + crit_badge.width() < title_width;
    let descendant_badge = if show_descendant_badge {
        descendant_badge
    } else {
        String::new()
    };
    let badge_width = descendant_badge.width() + crit_badge.width();
    let content_width = title_width.saturating_sub(badge_width);

    let (mut spans, content_width_used) =
        build_breadcrumb_title_spans(entry, app, own_title, content_width, query, title_style);

    let mut used_width = content_width_used;
    if !descendant_badge.is_empty() {
        used_width += descendant_badge.width();
        spans.push(Span::styled(descendant_badge, dim_style));
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

/// Returns the display width actually used (not padded) so the caller can
/// append the descendant-count badge directly after this content and pad
/// only once both are known.
///
/// Builds the session breadcrumb prefix and title as separate style regions.
/// `truncate` only ever cuts from the end, so the breadcrumb survives intact
/// whenever the cut lands in the title.
fn build_breadcrumb_title_spans(
    entry: &SessionRowEntry,
    app: &App,
    own_title: &str,
    max_width: usize,
    query: &str,
    title_style: Style,
) -> (Vec<Span<'static>>, usize) {
    let dim_style = Style::default().fg(DIM_FG);
    let breadcrumb_prefix = entry.breadcrumb_ancestor.map(|parent| {
        let parent_title = app
            .get_cached_title(&parent.session_id)
            .map(String::from)
            .unwrap_or_else(|| get_title_display_name_fallback(parent));
        format!("{parent_title} \u{203a} ")
    });

    let combined = format!("{}{own_title}", breadcrumb_prefix.as_deref().unwrap_or(""));
    let truncated = truncate(&combined, max_width);
    let width = truncated.width();
    let truncated_chars: Vec<char> = truncated.chars().collect();

    let breadcrumb_boundary = breadcrumb_prefix
        .as_deref()
        .map_or(0, |p| p.chars().count());

    let mut spans = Vec::new();
    let mut cursor = 0usize;
    let regions = [
        (breadcrumb_boundary, dim_style),
        (truncated_chars.len(), title_style),
    ];
    for (end, style) in regions {
        let end = end.min(truncated_chars.len());
        if end > cursor {
            let text: String = truncated_chars[cursor..end].iter().collect();
            spans.extend(highlight_matches(&text, query, style));
            cursor = end;
        }
    }

    (spans, width)
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
    #[case::needs_you(
        Section::NeedsYou,
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    )]
    #[case::running(
        Section::Running,
        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
    )]
    #[case::unread(
        Section::Unread,
        Style::default().fg(DIM_FG).add_modifier(Modifier::BOLD)
    )]
    #[case::idle(
        Section::Idle,
        Style::default().fg(DIM_FG).add_modifier(Modifier::BOLD)
    )]
    fn test_header_style(#[case] kind: Section, #[case] expected: Style) {
        assert_eq!(header_style(kind), expected);
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
    #[case::zero_shows_no_badge(0, "")]
    #[case::one(1, " \u{25b8}1")]
    #[case::two_digits(42, " \u{25b8}42")]
    fn test_descendant_badge_text(#[case] descendant_count: usize, #[case] expected: &str) {
        assert_eq!(descendant_badge_text(descendant_count), expected);
    }
}
