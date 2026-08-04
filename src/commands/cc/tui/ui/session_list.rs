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
};

use super::helpers::{
    get_session_info, get_title_display_name_fallback, highlight_matches, status_color, truncate,
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
/// Number of dashes kept after a section header's collapse/expand hint, so
/// the rule visibly continues past it instead of stopping dead.
const HEADER_HINT_TRAILING_DASHES: usize = 4;
/// Absolute column where a `WaitingInput` session's question line starts:
/// same width as the marker + status + repo columns combined, so the
/// question sits under the title column rather than the repo column.
const WAITING_QUESTION_INDENT: usize = MARKER_WIDTH + STATUS_COLUMN_WIDTH + REPO_COLUMN_WIDTH;
/// Below this age, the time column renders in the default (bright)
/// foreground; at or above it, it dims to `Color::DarkGray`. Independent of
/// status color, so a stale RUNNING session's time still reads as stale.
const RECENT_TIME_THRESHOLD_SECS: i64 = 3600;

/// Renders the session list grouped into fixed status sections (NEEDS YOU /
/// RUNNING / UNREAD / PAUSED-STOPPED), each session as one row (two for
/// `WaitingInput`), with fixed-width columns so the time column aligns
/// vertically across every row regardless of section.
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
        let empty_message = Paragraph::new(message).style(Style::default().fg(Color::DarkGray));
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

    let rows = build_session_rows(&filtered_sessions, app.paused_stopped_expanded);

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
            SessionRow::CollapsedSummary(sessions) => {
                build_collapsed_summary_item(sessions, app, term_width)
            }
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
        Section::Unread | Section::Idle => base.fg(Color::DarkGray),
    }
}

/// Renders a section header as a horizontal rule with the label inline,
/// e.g. `── RUNNING (3) ──...──`. For the collapsible section, splices a
/// right-ish hint before a short trailing dash run so the rule visibly
/// continues past it.
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

    let content = match header.collapsible {
        Some(is_expanded) => {
            let hint = if is_expanded {
                "Space: collapse"
            } else {
                "Space: expand"
            };
            let hint_str = format!(" {hint} ");
            let used = prefix.width() + hint_str.width() + HEADER_HINT_TRAILING_DASHES;
            let leading_dashes = content_width.saturating_sub(used);
            format!(
                "{prefix}{}{hint_str}{}",
                "─".repeat(leading_dashes),
                "─".repeat(HEADER_HINT_TRAILING_DASHES)
            )
        }
        None => {
            let dashes = content_width.saturating_sub(prefix.width());
            format!("{prefix}{}", "─".repeat(dashes))
        }
    };

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

    let symbol = session.display_symbol();
    let status_style = Style::default().fg(status_color(session.status));

    let repo_name = app
        .get_cached_worktree_labels(&session.cwd)
        .map_or("", |(repo, _)| repo);
    let repo_text = get_session_info(session, repo_name);
    let repo_col = pad_to_width(&truncate(&repo_text, REPO_COLUMN_WIDTH), REPO_COLUMN_WIDTH);

    let own_title = app
        .get_cached_title(&session.session_id)
        .map(String::from)
        .unwrap_or_else(|| get_title_display_name_fallback(session));

    let title_width = title_column_width(term_width);
    let title_spans = build_title_spans(entry, app, &own_title, title_width, query, is_idle);

    let time_text = format_compact_time(session.updated_at, now);
    let time_col = pad_left_to_width(&time_text, TIME_COLUMN_WIDTH);
    let seconds_since_update = now.signed_duration_since(session.updated_at).num_seconds();
    let time_style = if seconds_since_update < RECENT_TIME_THRESHOLD_SECS {
        Style::default()
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let mut spans = vec![
        Span::styled(symbol, status_style),
        Span::raw(" "),
        Span::styled(repo_col, Style::default().fg(Color::DarkGray)),
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
            Span::styled(truncated_quoted, Style::default().fg(Color::DarkGray)),
        ]));
    }

    ListItem::new(lines)
}

/// Builds the title column's spans: an optional dim `"{parent} › "`
/// breadcrumb followed by the session's own title, truncated together as
/// one string (so the breadcrumb never pushes the title off-screen) and
/// padded back out to `title_width` so the time column stays aligned.
fn build_title_spans(
    entry: &SessionRowEntry,
    app: &App,
    own_title: &str,
    title_width: usize,
    query: &str,
    is_idle: bool,
) -> Vec<Span<'static>> {
    let title_style = if is_idle {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    };
    let dim_style = Style::default().fg(Color::DarkGray);

    let Some(parent) = entry.breadcrumb_ancestor else {
        let padded = pad_to_width(&truncate(own_title, title_width), title_width);
        return highlight_matches(&padded, query, title_style);
    };

    let parent_title = app
        .get_cached_title(&parent.session_id)
        .map(String::from)
        .unwrap_or_else(|| get_title_display_name_fallback(parent));
    let prefix = format!("{parent_title} \u{203a} ");
    let combined = format!("{prefix}{own_title}");
    let truncated = truncate(&combined, title_width);
    let padded = pad_to_width(&truncated, title_width);

    // `truncate` only ever cuts from the end, so the breadcrumb survives
    // intact unless the column is too narrow even for it -- in that rare
    // case, render the whole (truncated) row dim rather than splitting.
    let boundary_chars = prefix.chars().count();
    let padded_chars: Vec<char> = padded.chars().collect();
    if padded_chars.len() <= boundary_chars {
        return highlight_matches(&padded, query, dim_style);
    }

    let breadcrumb_part: String = combined.chars().take(boundary_chars).collect();
    let title_part: String = padded_chars[boundary_chars..].iter().collect();
    let mut spans = highlight_matches(&breadcrumb_part, query, dim_style);
    spans.extend(highlight_matches(&title_part, query, title_style));
    spans
}

/// Resolves a session's repo name for the collapsed-summary grouping: the
/// cached repo label when available, otherwise the same cwd-basename
/// fallback `get_session_info` would produce.
fn resolve_repo_label(session: &Session, app: &App) -> String {
    match app.get_cached_worktree_labels(&session.cwd) {
        Some((repo, _worktree)) if !repo.is_empty() => repo.to_string(),
        _ => get_session_info(session, ""),
    }
}

/// Renders the collapsed Paused/Stopped section as a single summary row:
/// `"{repo} ×{count} · {title} / {title} / +{n}"` (or `"{count} sessions"`
/// when the group spans more than one repo).
fn build_collapsed_summary_item(
    sessions: &[&Session],
    app: &App,
    term_width: usize,
) -> ListItem<'static> {
    let content_width = term_width.saturating_sub(MARKER_WIDTH);
    let dim_style = Style::default().fg(Color::DarkGray);

    let labels: Vec<String> = sessions
        .iter()
        .map(|s| resolve_repo_label(s, app))
        .collect();
    let same_repo = labels
        .first()
        .is_some_and(|first| labels.iter().all(|l| l == first));
    let prefix = if same_repo {
        format!("{} \u{d7}{}", labels[0], sessions.len())
    } else {
        format!("{} sessions", sessions.len())
    };

    let titles: Vec<String> = sessions
        .iter()
        .take(2)
        .map(|s| {
            app.get_cached_title(&s.session_id)
                .map(String::from)
                .unwrap_or_else(|| get_title_display_name_fallback(s))
        })
        .collect();
    let mut titles_text = titles.join(" / ");
    if sessions.len() > 2 {
        titles_text.push_str(&format!(" / +{}", sessions.len() - 2));
    }

    let full = format!("  {prefix} \u{b7} {titles_text}");
    let truncated = truncate(&full, content_width);

    ListItem::new(vec![Line::from(Span::styled(truncated, dim_style))])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::tui::ui::test_support::{
        create_test_session, render_buffer, render_to_string, render_to_string_with,
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
        Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)
    )]
    #[case::idle(
        Section::Idle,
        Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)
    )]
    fn test_header_style(#[case] kind: Section, #[case] expected: Style) {
        assert_eq!(header_style(kind), expected);
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
        assert_ne!(buffer[(79, 2)].fg, Color::DarkGray);
        assert_eq!(buffer[(79, 3)].fg, Color::DarkGray);
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
        assert_eq!(buffer[(3, 2)].fg, Color::DarkGray);
        assert_eq!(buffer[(3, 6)].fg, Color::DarkGray);
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
             ● project         project                                              just now




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
        // the (by-default expanded) PAUSED section -- crossing every section
        // boundary, so a cumulative off-by-N from repeated separators would
        // show up here.
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

             ── PAUSED (1) ──────────────────────────────────────────── Space: collapse ────
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
    fn test_render_collapsed_paused_summary_single_repo() {
        let now = Utc::now();

        let mut paused1 = create_test_session("paused1");
        paused1.status = SessionStatus::Paused;
        let mut paused2 = create_test_session("paused2");
        paused2.status = SessionStatus::Paused;

        let sessions = vec![paused1, paused2];
        let output = render_to_string_with(&sessions, None, now, 80, 9, |app| {
            app.paused_stopped_expanded = false;
        });

        let expected = indoc! {"
             cc watch                                       0 needs you · 0 running · 2 idle
            ── PAUSED (2) ────────────────────────────────────────────── Space: expand ────
              project ×2 · project / project





             ?: keys   /: search   Tab: worktree   q: quit"};

        assert_eq!(output, expected);
    }

    #[test]
    fn test_render_expanded_paused_section_shows_individual_rows() {
        let now = Utc::now();

        let mut paused1 = create_test_session("paused1");
        paused1.status = SessionStatus::Paused;
        let mut paused2 = create_test_session("paused2");
        paused2.status = SessionStatus::Paused;

        let sessions = vec![paused1, paused2];
        // Expanded by default; no setup override needed.
        let output = render_to_string(&sessions, None, now, 80, 10);

        let expected = indoc! {"
             cc watch                                       0 needs you · 0 running · 2 idle
            ── PAUSED (2) ──────────────────────────────────────────── Space: collapse ────
            ⏸ project         project                                              just now
            ⏸ project         project                                              just now





             ?: keys   /: search   Tab: worktree   q: quit"};

        assert_eq!(output, expected);
    }
}
