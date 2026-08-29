use crate::commands::cc::types::{Session, SessionStatus};
use chrono::{DateTime, Utc};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use crate::commands::cc::tui::app::{App, AppMode};
use crate::commands::cc::tui::session_rows::{
    Section, SectionHeaderRow, SessionRow, SessionRowEntry, build_session_rows, is_idle_session,
    is_related_task, kin_relation,
};

use super::helpers::{
    DIM_FG, get_session_info, get_title_display_name_fallback, highlight_matches, kin_color,
    status_color, truncate,
};

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
/// Fixed width of the right-aligned time column.
const TIME_COLUMN_WIDTH: usize = 9;
/// Floor for the variable-width title column so it never collapses to
/// nothing on very narrow terminals.
const MIN_TITLE_WIDTH: usize = 10;
/// Absolute column where a `WaitingInput` session's question line starts:
/// same width as the marker + status + repo columns combined, so the
/// question sits under the title column rather than the repo column.
const WAITING_QUESTION_INDENT: usize = MARKER_WIDTH + STATUS_COLUMN_WIDTH + REPO_COLUMN_WIDTH;
/// Below this age, the time column renders in the default (bright)
/// foreground; at or above it, it dims to `DIM_FG`. Independent of status
/// color, so a stale RUNNING session's time still reads as stale.
const RECENT_TIME_THRESHOLD_SECS: i64 = 3600;

/// Renders the session list, grouped into fixed status sections (NEEDS YOU /
/// RUNNING / UNREAD / PAUSED-STOPPED), each session as one row (two for
/// `WaitingInput`), with fixed-width columns so the time column aligns
/// vertically across every row regardless of section. A session linked to a
/// tq task (`app.task_by_session`) gets a `#<number> <title> › ` prefix
/// ahead of its usual breadcrumb/title, dimmed unless its task is related to
/// the cursor row's task (see [`is_related_task`]).
pub(super) fn render_session_list(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    now: DateTime<Utc>,
) {
    let filtered_sessions: Vec<&Session> = app.filtered_sessions();

    if filtered_sessions.is_empty() {
        let message = if app.mode == AppMode::Search {
            format!("  No sessions match \"{}\"", app.search_query)
        } else if app.has_filter() {
            let mut parts = Vec::new();
            if let Some(status) = app.status_filter {
                parts.push(format!("status:{}", status.display_name()));
            }
            if !app.confirmed_query.is_empty() {
                parts.push(format!("\"{}\"", app.confirmed_query));
            }
            format!("  No sessions match {}", parts.join(" + "))
        } else {
            "  No active Claude Code sessions.".to_string()
        };
        let empty_message = Paragraph::new(message).style(Style::default().fg(DIM_FG));
        frame.render_widget(empty_message, area);
        return;
    }

    let term_width = area.width as usize;

    // Determine the active search query for highlighting.
    // Clone to avoid borrowing app across the mutable cache update.
    let query = if app.mode == AppMode::Search {
        app.search_query.clone()
    } else {
        app.confirmed_query.clone()
    };

    let rows = build_session_rows(&filtered_sessions, &app.task_by_session);

    // Build list items and owned row ids from the same `rows`, then drop
    // `rows`/`filtered_sessions` (which borrow `app`) before mutating app.
    //
    // `items` must stay in exact 1:1 correspondence with `rows` (same
    // length, same index for each entry): `list_state` indices select into
    // `items`, while `row_ids`/`app.row_sessions` are indexed by `rows`
    // position. A separate `ListItem` for the inter-section blank line
    // would desync the two spaces, so the blank line is instead prepended
    // to the following header's own `ListItem`.
    let mut row_ids: Vec<Option<String>> = Vec::with_capacity(rows.len());
    let mut items: Vec<ListItem> = Vec::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        row_ids.push(row.session_id().map(String::from));

        let item = match row {
            SessionRow::SectionHeader(header) => build_header_item(header, term_width, i > 0),
            SessionRow::Session(entry) => build_session_item(entry, app, now, term_width, &query),
        };
        items.push(item);
    }
    drop(rows);
    drop(filtered_sessions);

    let row_id_refs: Vec<Option<&str>> = row_ids.iter().map(|id| id.as_deref()).collect();
    app.update_row_order(&row_id_refs);

    let list = List::new(items)
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol(">");

    frame.render_stateful_widget(list, area, &mut app.list_state);
}

/// Variable width of the title column: whatever's left after the fixed
/// marker/status/repo/time columns, floored so it never disappears.
fn title_column_width(term_width: usize) -> usize {
    term_width
        .saturating_sub(MARKER_WIDTH + STATUS_COLUMN_WIDTH + REPO_COLUMN_WIDTH + TIME_COLUMN_WIDTH)
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

/// Task title-prefix style: plain/default when the row's task is related to
/// the cursor row's task (see [`is_related_task`]), `DIM_FG` otherwise. A
/// channel separate from `own_title_style`'s `kin_color` -- session kinship
/// colors the title, task kinship only ever dims or un-dims this prefix.
fn task_prefix_style(is_related: bool) -> Style {
    if is_related {
        Style::default()
    } else {
        Style::default().fg(DIM_FG)
    }
}

/// Renders a section header as a horizontal rule with the label inline,
/// e.g. `── RUNNING (3) ──...──`.
///
/// `with_leading_blank` prepends a blank line as a visual separator from
/// the previous section (every header but the very first one in the list).
/// It lives inside this `ListItem` rather than as a standalone item so that
/// `items` stays index-aligned with `rows` (see the caller).
fn build_header_item(
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
fn build_session_item(
    entry: &SessionRowEntry,
    app: &App,
    now: DateTime<Utc>,
    term_width: usize,
    query: &str,
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
    let task_prefix = entry.task.as_ref().map(|task| {
        let related = is_related_task(cursor_task, Some(task));
        (
            format!("#{} {} \u{203a} ", task.task_number, task.task_title),
            task_prefix_style(related),
        )
    });

    let title_width = title_column_width(term_width);
    let title_style = own_title_style(is_idle, title_kin_color);
    let title_spans = build_title_spans(
        entry,
        app,
        &own_title,
        title_width,
        query,
        title_style,
        task_prefix,
    );

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
        let quoted_width = term_width.saturating_sub(WAITING_QUESTION_INDENT);
        let truncated_quoted = truncate(&quoted, quoted_width);
        // ratatui reserves the marker column on every line of a multi-line
        // `ListItem`, not just the first, so our own content only needs to
        // cover the remaining status+repo width to reach `WAITING_QUESTION_INDENT`.
        lines.push(Line::from(vec![
            Span::raw(" ".repeat(WAITING_QUESTION_INDENT - MARKER_WIDTH)),
            Span::styled(truncated_quoted, Style::default().fg(DIM_FG)),
        ]));
    }

    ListItem::new(lines)
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
    task_prefix: Option<(String, Style)>,
) -> Vec<Span<'static>> {
    let dim_style = Style::default().fg(DIM_FG);
    let badge = descendant_badge_text(entry.descendant_count);
    let badge = if badge.width() < title_width {
        badge
    } else {
        String::new()
    };
    let content_width = title_width - badge.width();

    let (mut spans, content_width_used) = build_breadcrumb_title_spans(
        entry,
        app,
        own_title,
        content_width,
        query,
        title_style,
        task_prefix,
    );

    let mut used_width = content_width_used;
    if !badge.is_empty() {
        used_width += badge.width();
        spans.push(Span::styled(badge, dim_style));
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
/// Builds up to three style regions, left to right: `task_prefix` (kinship
/// brightness, see [`task_prefix_style`]), the session breadcrumb prefix
/// (always `dim_style`, unrelated to task kinship), then `own_title`
/// (`title_style`, carrying session-kinship `kin_color`). `truncate` only
/// ever cuts from the end, so an earlier region survives intact whenever the
/// cut lands in a later one -- only the region the cut actually lands in
/// (and none after it) loses content.
fn build_breadcrumb_title_spans(
    entry: &SessionRowEntry,
    app: &App,
    own_title: &str,
    max_width: usize,
    query: &str,
    title_style: Style,
    task_prefix: Option<(String, Style)>,
) -> (Vec<Span<'static>>, usize) {
    let dim_style = Style::default().fg(DIM_FG);
    let (task_prefix_text, task_prefix_style) =
        task_prefix.unwrap_or_else(|| (String::new(), dim_style));
    let breadcrumb_prefix = entry.breadcrumb_ancestor.map(|parent| {
        let parent_title = app
            .get_cached_title(&parent.session_id)
            .map(String::from)
            .unwrap_or_else(|| get_title_display_name_fallback(parent));
        format!("{parent_title} \u{203a} ")
    });

    let combined = format!(
        "{task_prefix_text}{}{own_title}",
        breadcrumb_prefix.as_deref().unwrap_or("")
    );
    let truncated = truncate(&combined, max_width);
    let width = truncated.width();
    let truncated_chars: Vec<char> = truncated.chars().collect();

    let task_boundary = task_prefix_text.chars().count();
    let breadcrumb_boundary = task_boundary
        + breadcrumb_prefix
            .as_deref()
            .map_or(0, |p| p.chars().count());

    let mut spans = Vec::new();
    let mut cursor = 0usize;

    if task_boundary > cursor {
        let end = task_boundary.min(truncated_chars.len());
        let text: String = truncated_chars[cursor..end].iter().collect();
        spans.extend(highlight_matches(&text, query, task_prefix_style));
        cursor = end;
    }

    if breadcrumb_boundary > cursor {
        let end = breadcrumb_boundary.min(truncated_chars.len());
        let text: String = truncated_chars[cursor..end].iter().collect();
        spans.extend(highlight_matches(&text, query, dim_style));
        cursor = end;
    }

    if truncated_chars.len() > cursor {
        let text: String = truncated_chars[cursor..].iter().collect();
        spans.extend(highlight_matches(&text, query, title_style));
    }

    (spans, width)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::tui::session_rows::SessionTask;
    use crate::commands::cc::tui::ui::test_support::{
        create_test_session, render_buffer, render_buffer_with, render_to_string,
        render_to_string_with,
    };
    use indoc::indoc;
    use rstest::{fixture, rstest};

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

    #[test]
    fn test_time_column_dims_after_one_hour_independent_of_status() {
        let now = Utc::now();

        let mut recent = create_test_session("recent");
        recent.status = SessionStatus::Running;
        recent.updated_at = now - chrono::Duration::minutes(5);

        let mut stale = create_test_session("stale");
        stale.status = SessionStatus::Running;
        stale.updated_at = now - chrono::Duration::hours(3);

        let sessions = vec![recent, stale];
        // A `Some` selection is required for ratatui's `List` to reserve
        // the highlight-symbol marker column at all (with no selection,
        // every row shifts one column left) -- match the column math every
        // other row test in this file relies on.
        let buffer = render_buffer(&sessions, Some(2), now, 80, 12);

        // Row 0 is the chrome header, row 1 the "RUNNING (2)" section
        // header, rows 2/3 the two sessions. Column 79 (the last column)
        // always falls inside the right-aligned time span regardless of
        // the rendered text's length, since the whole padded field shares
        // one style.
        assert_ne!(buffer[(79, 2)].fg, DIM_FG);
        assert_eq!(buffer[(79, 3)].fg, DIM_FG);
    }

    #[test]
    fn test_repo_column_is_always_dim() {
        let now = Utc::now();

        let mut waiting = create_test_session("waiting");
        waiting.status = SessionStatus::WaitingInput;
        let mut running = create_test_session("running");
        running.status = SessionStatus::Running;

        let sessions = vec![waiting, running];
        let buffer = render_buffer(&sessions, Some(2), now, 80, 12);

        // Row 2 is "waiting"'s row, row 6 is "running"'s row (accounting
        // for the question line and blank separator in between). Column 3
        // is the first character of the repo column for either row.
        assert_eq!(buffer[(3, 2)].fg, DIM_FG);
        assert_eq!(buffer[(3, 6)].fg, DIM_FG);
    }

    // =========================================================================
    // Full-screen integration tests (TRIAGE inbox layout)
    // =========================================================================

    #[test]
    fn test_render_only_running_section_shows_no_other_headers() {
        let now = Utc::now();

        let mut session = create_test_session("s1");
        session.updated_at = now;
        session.status = SessionStatus::Running;

        let sessions = vec![session];
        let output = render_to_string(&sessions, Some(1), now, 80, 9);

        let expected = indoc! {"
             cc watch                                       0 needs you · 1 running · 0 idle
             ── RUNNING (1) ────────────────────────────────────────────────────────────────
            >● project         project                                              just now





             ?: keys   /: search   Tab: worktree   q: quit"};

        assert_eq!(output, expected);
    }

    fn linked_task(
        task_number: u32,
        task_title: &str,
        parent_task_id: Option<&str>,
    ) -> SessionTask {
        SessionTask {
            task_id: format!("task-{task_number}"),
            task_number,
            task_title: task_title.to_string(),
            parent_task_id: parent_task_id.map(String::from),
        }
    }

    #[test]
    fn test_render_task_prefix_appears_before_breadcrumb_and_title() {
        let now = Utc::now();

        let mut session = create_test_session("s1");
        session.updated_at = now;
        session.status = SessionStatus::Running;

        let sessions = vec![session];
        let output = render_to_string_with(&sessions, Some(1), now, 80, 9, |app| {
            app.task_by_session
                .insert("s1".to_string(), linked_task(42, "Fix the bug", None));
        });

        let expected = indoc! {"
             cc watch                                       0 needs you · 1 running · 0 idle
             ── RUNNING (1) ────────────────────────────────────────────────────────────────
            >● project         #42 Fix the bug › project                            just now





             ?: keys   /: search   Tab: worktree   q: quit"};

        assert_eq!(output, expected);
    }

    #[test]
    fn test_render_task_kin_highlight_dims_unrelated_task_prefix() {
        let now = Utc::now();

        // "cursor" and "same_task" share task #42; "other_task" is linked to
        // a distinct, unrelated task #57.
        let mut cursor = create_test_session("cursor");
        cursor.updated_at = now;
        cursor.status = SessionStatus::Running;
        let mut same_task = create_test_session("same_task");
        same_task.updated_at = now;
        same_task.status = SessionStatus::Running;
        let mut other_task = create_test_session("other_task");
        other_task.updated_at = now;
        other_task.status = SessionStatus::Running;

        let sessions = vec![cursor, same_task, other_task];
        // Row y: chrome(y0), "RUNNING (3)" header(y1), cursor(y2),
        // same_task(y3), other_task(y4). list_state index 1 (cursor) is
        // the first *selectable* row, i.e. the row right after the header.
        let buffer = render_buffer_with(&sessions, Some(1), now, 80, 12, |app| {
            app.task_by_session
                .insert("cursor".to_string(), linked_task(42, "Fix the bug", None));
            app.task_by_session.insert(
                "same_task".to_string(),
                linked_task(42, "Fix the bug", None),
            );
            app.task_by_session.insert(
                "other_task".to_string(),
                linked_task(57, "Unrelated bug", None),
            );
        });

        // Column 19 is where the title column (and so the task-prefix, when
        // present) starts on every row -- see `WAITING_QUESTION_INDENT`.
        assert_ne!(buffer[(19, 2)].fg, DIM_FG);
        assert_ne!(buffer[(19, 3)].fg, DIM_FG);
        assert_eq!(buffer[(19, 4)].fg, DIM_FG);
    }

    #[test]
    fn test_render_background_session_shows_distinct_symbol_and_color() {
        // Persisted `status` is `Stopped` (main loop idle), but `section_of`
        // still groups a pending background task into RUNNING, so the
        // glyph/color must distinguish "main loop idle, background task in
        // flight" from a session actually running.
        let now = Utc::now();

        let mut session = create_test_session("s1");
        session.updated_at = now;
        session.status = SessionStatus::Stopped;
        session.pending_bg_task_ids.insert("bg-1".to_string());

        let sessions = vec![session];
        let output = render_to_string(&sessions, Some(1), now, 80, 9);

        let expected = indoc! {"
             cc watch                                       0 needs you · 1 running · 0 idle
             ── RUNNING (1) ────────────────────────────────────────────────────────────────
            >◎ project         project                                              just now





             ?: keys   /: search   Tab: worktree   q: quit"};

        assert_eq!(output, expected);

        let buffer = render_buffer(&sessions, Some(1), now, 80, 9);
        assert_eq!(buffer[(1, 2)].fg, Color::Cyan);
    }

    #[test]
    fn test_render_needs_you_section_shows_question_line() {
        let now = Utc::now();

        let mut session = create_test_session("s1");
        session.updated_at = now;
        session.status = SessionStatus::WaitingInput;
        session.current_tool = Some("Which approach do you prefer?".to_string());

        let sessions = vec![session];
        let output = render_to_string(&sessions, Some(1), now, 80, 10);

        let expected = indoc! {"
             cc watch                                       1 needs you · 0 running · 0 idle
             ── NEEDS YOU ──────────────────────────────────────────────────────────────────
            >◐ project         project                                              just now
                               “Which approach do you prefer?”





             ?: keys   /: search   Tab: worktree   q: quit"};

        assert_eq!(output, expected);
    }

    #[test]
    fn test_render_waiting_session_with_no_question_shows_empty_quotes() {
        let now = Utc::now();

        let mut session = create_test_session("s1");
        session.updated_at = now;
        session.status = SessionStatus::WaitingInput;
        session.current_tool = None;
        session.last_message = None;

        let sessions = vec![session];
        let output = render_to_string(&sessions, Some(1), now, 80, 10);

        let expected = indoc! {"
             cc watch                                       1 needs you · 0 running · 0 idle
             ── NEEDS YOU ──────────────────────────────────────────────────────────────────
            >◐ project         project                                              just now
                               “”





             ?: keys   /: search   Tab: worktree   q: quit"};

        assert_eq!(output, expected);
    }

    #[rstest]
    #[case::zero_shows_no_badge(0, "")]
    #[case::one(1, " \u{25b8}1")]
    #[case::two_digits(42, " \u{25b8}42")]
    fn test_descendant_badge_text(#[case] descendant_count: usize, #[case] expected: &str) {
        assert_eq!(descendant_badge_text(descendant_count), expected);
    }

    #[test]
    fn test_render_session_with_descendants_shows_count_badge_after_title() {
        let now = Utc::now();

        let mut root = create_test_session("root");
        root.updated_at = now;
        root.status = SessionStatus::Running;

        let mut child_a = create_test_session("child_a");
        child_a.updated_at = now;
        child_a.ancestor_session_ids = vec!["root".to_string()];
        child_a.status = SessionStatus::Running;

        let mut child_b = create_test_session("child_b");
        child_b.updated_at = now;
        child_b.ancestor_session_ids = vec!["root".to_string()];
        child_b.status = SessionStatus::Running;

        let sessions = vec![root, child_a, child_b];
        let output = render_to_string(&sessions, Some(1), now, 80, 10);

        let expected = indoc! {"
             cc watch                                       0 needs you · 3 running · 0 idle
             ── RUNNING (3) ────────────────────────────────────────────────────────────────
            >● project         project ▸2                                           just now
             ● project         project › project                                    just now
             ● project         project › project                                    just now




             ?: keys   /: search   Tab: worktree   q: quit"};

        assert_eq!(output, expected);
    }

    #[test]
    fn test_render_child_in_different_section_from_parent_shows_breadcrumb() {
        let now = Utc::now();

        let mut parent = create_test_session("parent");
        parent.updated_at = now;
        parent.status = SessionStatus::Running;

        let mut child = create_test_session("child");
        child.updated_at = now - chrono::Duration::minutes(2);
        child.ancestor_session_ids = vec!["parent".to_string()];
        child.status = SessionStatus::WaitingInput;
        child.current_tool = Some("Pick one".to_string());

        let sessions = vec![parent, child];
        let output = render_to_string(&sessions, Some(1), now, 80, 12);

        let expected = indoc! {"
             cc watch                                       1 needs you · 1 running · 0 idle
             ── NEEDS YOU ──────────────────────────────────────────────────────────────────
            >◐ project         project › project                                          2m
                               “Pick one”

             ── RUNNING (1) ────────────────────────────────────────────────────────────────
             ● project         project ▸1                                           just now




             ?: keys   /: search   Tab: worktree   q: quit"};

        assert_eq!(output, expected);
    }

    // =========================================================================
    // Cursor-position regression tests: exercise `select_next`/`select_previous`
    // themselves (not `list_state.select` injection) so a `ListItem` array
    // that desyncs from `rows` (e.g. reintroducing a standalone separator
    // `ListItem`) is caught by the render, not just by `App`-level state.
    // =========================================================================

    #[fixture]
    fn waiting_and_running_sessions() -> (DateTime<Utc>, Vec<Session>) {
        let now = Utc::now();

        let mut waiting = create_test_session("waiting");
        waiting.updated_at = now;
        waiting.status = SessionStatus::WaitingInput;
        waiting.current_tool = Some("Pick one".to_string());

        let mut running = create_test_session("running");
        running.updated_at = now;
        running.status = SessionStatus::Running;

        (now, vec![waiting, running])
    }

    #[rstest]
    fn test_select_next_across_two_sections_lands_on_session_row(
        waiting_and_running_sessions: (DateTime<Utc>, Vec<Session>),
    ) {
        let (now, sessions) = waiting_and_running_sessions;
        let output = render_to_string_with(&sessions, Some(1), now, 80, 12, |app| {
            app.select_next();
        });

        let expected = indoc! {"
             cc watch                                       1 needs you · 1 running · 0 idle
             ── NEEDS YOU ──────────────────────────────────────────────────────────────────
             ◐ project         project                                              just now
                               “Pick one”

             ── RUNNING (1) ────────────────────────────────────────────────────────────────
            >● project         project                                              just now




             ?: keys   /: search   Tab: worktree   q: quit"};

        assert_eq!(output, expected);
    }

    #[rstest]
    fn test_select_next_across_all_four_sections_lands_on_session_row(
        waiting_and_running_sessions: (DateTime<Utc>, Vec<Session>),
    ) {
        let (now, mut sessions) = waiting_and_running_sessions;

        let mut unread = create_test_session("unread");
        unread.updated_at = now;
        unread.status = SessionStatus::Stopped;
        unread.read_at = None;

        let mut paused = create_test_session("paused");
        paused.updated_at = now;
        paused.status = SessionStatus::Paused;

        sessions.push(unread);
        sessions.push(paused);
        // Start on "waiting" and step through RUNNING, UNREAD, all the way to
        // the PAUSED section -- crossing every section boundary, so a
        // cumulative off-by-N from repeated separators would show up here.
        let output = render_to_string_with(&sessions, Some(1), now, 80, 16, |app| {
            app.select_next();
            app.select_next();
            app.select_next();
        });

        let expected = indoc! {"
             cc watch                                       1 needs you · 1 running · 2 idle
             ── NEEDS YOU ──────────────────────────────────────────────────────────────────
             ◐ project         project                                              just now
                               “Pick one”

             ── RUNNING (1) ────────────────────────────────────────────────────────────────
             ● project         project                                              just now

             ── UNREAD (1) ─────────────────────────────────────────────────────────────────
             ✱ project         project                                              just now

             ── PAUSED (1) ─────────────────────────────────────────────────────────────────
            >⏸ project         project                                              just now


             ?: keys   /: search   Tab: worktree   q: quit"};

        assert_eq!(output, expected);
    }

    #[rstest]
    fn test_select_previous_wraps_backward_across_section_lands_on_session_row(
        waiting_and_running_sessions: (DateTime<Utc>, Vec<Session>),
    ) {
        let (now, sessions) = waiting_and_running_sessions;
        // Starting on the first selectable row ("waiting") and going
        // backward must wrap to the last selectable row ("running"), not to
        // the RUNNING section header.
        let output = render_to_string_with(&sessions, Some(1), now, 80, 12, |app| {
            app.select_previous();
        });

        let expected = indoc! {"
             cc watch                                       1 needs you · 1 running · 0 idle
             ── NEEDS YOU ──────────────────────────────────────────────────────────────────
             ◐ project         project                                              just now
                               “Pick one”

             ── RUNNING (1) ────────────────────────────────────────────────────────────────
            >● project         project                                              just now




             ?: keys   /: search   Tab: worktree   q: quit"};

        assert_eq!(output, expected);
    }

    #[test]
    fn test_render_paused_section_shows_individual_session_rows() {
        let now = Utc::now();

        let mut paused1 = create_test_session("paused1");
        paused1.status = SessionStatus::Paused;
        let mut paused2 = create_test_session("paused2");
        paused2.status = SessionStatus::Paused;

        let sessions = vec![paused1, paused2];
        let output = render_to_string(&sessions, None, now, 80, 10);

        let expected = indoc! {"
             cc watch                                       0 needs you · 0 running · 2 idle
            ── PAUSED (2) ─────────────────────────────────────────────────────────────────
            ⏸ project         project                                              just now
            ⏸ project         project                                              just now





             ?: keys   /: search   Tab: worktree   q: quit"};

        assert_eq!(output, expected);
    }

    // =========================================================================
    // Kin highlighting: cursor-relative ancestor/descendant/collateral coloring
    // =========================================================================

    #[rstest]
    // `e` is selected; `a`..`d` are its ancestors at increasing distance.
    // `a` sits one generation past `MAX_KIN_DISTANCE` and must render
    // uncolored, same as the cursor row `e` itself. Rows: chrome(y0),
    // section header(y1), a(y2) b(y3) c(y4) d(y5) e(y6), in input order.
    #[case::beyond_cap_ancestor_a(2, 19, Color::Reset)]
    #[case::great_grandparent_b(3, 29, Color::Indexed(146))]
    #[case::grandparent_c(4, 29, Color::Indexed(111))]
    #[case::parent_d(5, 29, Color::Indexed(39))]
    #[case::cursor_row_e(6, 29, Color::Reset)]
    fn test_render_kin_highlight_ancestor_ramp_and_cap(
        #[case] row_y: u16,
        #[case] col_x: u16,
        #[case] expected_fg: Color,
    ) {
        let now = Utc::now();
        let mut a = create_test_session("a");
        a.updated_at = now;
        let mut b = create_test_session("b");
        b.updated_at = now;
        b.ancestor_session_ids = vec!["a".to_string()];
        let mut c = create_test_session("c");
        c.updated_at = now;
        c.ancestor_session_ids = vec!["a".to_string(), "b".to_string()];
        let mut d = create_test_session("d");
        d.updated_at = now;
        d.ancestor_session_ids = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let mut e = create_test_session("e");
        e.updated_at = now;
        e.ancestor_session_ids = vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
        ];

        let sessions = vec![a, b, c, d, e];
        // list_state index: header=0, a=1, b=2, c=3, d=4, e=5 -- select "e".
        let buffer = render_buffer(&sessions, Some(5), now, 80, 12);

        assert_eq!(buffer[(col_x, row_y)].fg, expected_fg);
    }

    #[rstest]
    // `selected` is the cursor. `sibling` shares its immediate parent
    // (distance 1); `cousin` shares its grandparent via a different parent
    // (distance 2); `great_uncle` shares only the great-grandparent root,
    // one generation past `MAX_COLLATERAL_KIN_DISTANCE`, and must render
    // uncolored, same as the cursor row itself. Rows: chrome(y0),
    // header(y1), root(y2) gp_a(y3) gp_b(y4) parent_a(y5) parent_a2(y6)
    // selected(y7) sibling(y8) cousin(y9), in input order.
    #[case::beyond_cap_great_uncle(4, 29, Color::Reset)]
    #[case::cursor_row_selected(7, 29, Color::Reset)]
    #[case::sibling(8, 29, Color::Indexed(129))]
    #[case::cousin(9, 29, Color::Indexed(135))]
    fn test_render_kin_highlight_collateral_ramp_and_cap(
        #[case] row_y: u16,
        #[case] col_x: u16,
        #[case] expected_fg: Color,
    ) {
        let now = Utc::now();
        let mut root = create_test_session("root");
        root.updated_at = now;
        let mut gp_a = create_test_session("gp_a");
        gp_a.updated_at = now;
        gp_a.ancestor_session_ids = vec!["root".to_string()];
        let mut gp_b = create_test_session("gp_b");
        gp_b.updated_at = now;
        gp_b.ancestor_session_ids = vec!["root".to_string()];
        let mut parent_a = create_test_session("parent_a");
        parent_a.updated_at = now;
        parent_a.ancestor_session_ids = vec!["root".to_string(), "gp_a".to_string()];
        let mut parent_a2 = create_test_session("parent_a2");
        parent_a2.updated_at = now;
        parent_a2.ancestor_session_ids = vec!["root".to_string(), "gp_a".to_string()];
        let mut selected = create_test_session("selected");
        selected.updated_at = now;
        selected.ancestor_session_ids = vec![
            "root".to_string(),
            "gp_a".to_string(),
            "parent_a".to_string(),
        ];
        let mut sibling = create_test_session("sibling");
        sibling.updated_at = now;
        sibling.ancestor_session_ids = vec![
            "root".to_string(),
            "gp_a".to_string(),
            "parent_a".to_string(),
        ];
        let mut cousin = create_test_session("cousin");
        cousin.updated_at = now;
        cousin.ancestor_session_ids = vec![
            "root".to_string(),
            "gp_a".to_string(),
            "parent_a2".to_string(),
        ];

        let sessions = vec![
            root, gp_a, gp_b, parent_a, parent_a2, selected, sibling, cousin,
        ];
        // list_state index: header=0, root=1, gp_a=2, gp_b=3, parent_a=4,
        // parent_a2=5, selected=6, sibling=7, cousin=8 -- select "selected".
        let buffer = render_buffer(&sessions, Some(6), now, 80, 20);

        assert_eq!(buffer[(col_x, row_y)].fg, expected_fg);
    }

    #[test]
    fn test_render_kin_highlight_search_highlight_overrides_kin_color() {
        let now = Utc::now();
        let mut root = create_test_session("root");
        root.updated_at = now;
        let mut child = create_test_session("child");
        child.updated_at = now;
        child.ancestor_session_ids = vec!["root".to_string()];

        let sessions = vec![root, child];
        // list_state index: header=0, root=1 (selected/cursor), child=2.
        // A non-empty `confirmed_query` makes the chrome show a top filter
        // bar, so rows shift down by one: chrome(y0), filter bar(y1),
        // section header(y2), root(y3), child(y4).
        let buffer = render_buffer_with(&sessions, Some(1), now, 80, 10, |app| {
            app.confirmed_query = "project".to_string();
        });

        // child's own title (after its "project › " breadcrumb) starts at
        // column 29, row 4. It's a direct descendant of the cursor (pink
        // kin color), but the query "project" matches it too, and search
        // hits must win over kin coloring.
        assert_eq!(buffer[(29, 4)].fg, Color::Yellow);
    }
}
