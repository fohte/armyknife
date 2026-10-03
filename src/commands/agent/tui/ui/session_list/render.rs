use crate::commands::agent::types::Session;
use chrono::{DateTime, Utc};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    widgets::{List, ListItem, Paragraph},
};

use crate::commands::agent::tui::app::{App, AppMode};
use crate::commands::agent::tui::session_rows::{SessionRow, build_session_rows};

use super::super::helpers::DIM_FG;
use super::items::{build_header_item, build_session_item};
/// Renders the session list, grouped into fixed status sections (NEEDS YOU /
/// RUNNING / UNREAD / PAUSED-STOPPED), each session as one row (two for
/// `WaitingInput`), with fixed-width columns so the time column aligns
/// vertically across every row regardless of section. A session linked to a
/// tq task (`app.task_by_session`) gets a fixed-width task-number column,
/// dimmed unless its task is related to the cursor row's task (see
/// [`crate::commands::agent::tui::session_rows::is_related_task`]).
pub(in crate::commands::agent::tui::ui) fn render_session_list(
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
    let linked_session_ids = app.linked_session_ids_for_sidebar_cursor();

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
            SessionRow::Session(entry) => build_session_item(
                entry,
                app,
                now,
                term_width,
                &query,
                linked_session_ids.contains(&entry.session.session_id),
            ),
        };
        items.push(item);
    }
    drop(rows);
    drop(filtered_sessions);

    let row_id_refs: Vec<Option<&str>> = row_ids.iter().map(|id| id.as_deref()).collect();
    app.update_row_order(&row_id_refs);

    let selected_session_is_linked = app
        .selected_session()
        .is_some_and(|session| linked_session_ids.contains(&session.session_id));
    let list = List::new(items)
        .highlight_style(if app.sidebar_focused {
            if selected_session_is_linked {
                Style::default().bg(Color::Indexed(236))
            } else {
                Style::default()
            }
        } else {
            Style::default().bg(Color::DarkGray)
        })
        .highlight_symbol(if app.sidebar_focused { "›" } else { ">" });

    frame.render_stateful_widget(list, area, &mut app.list_state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::tui::session_rows::SessionTask;
    use crate::commands::agent::tui::ui::helpers::DIM_FG;
    use crate::commands::agent::tui::ui::test_support::{
        create_test_session, render_buffer, render_buffer_with, render_to_string,
        render_to_string_with, render_to_string_with_agent_config,
    };
    use crate::commands::agent::types::SessionStatus;
    use crate::shared::config::{AgentConfig, AgentWorkTypeColor, AgentWorkTypeConfig};
    use indoc::indoc;
    use ratatui::style::Modifier;
    use rstest::{fixture, rstest};
    use unicode_width::UnicodeWidthStr;

    use super::super::items::{
        MIN_WORK_TYPE_COLUMN_WIDTH, TASK_NUMBER_COLUMN_WIDTH, WAITING_QUESTION_BASE_INDENT,
    };

    const WAITING_QUESTION_INDENT: usize =
        WAITING_QUESTION_BASE_INDENT + MIN_WORK_TYPE_COLUMN_WIDTH;
    const TASK_NUMBER_COLUMN_START: usize =
        WAITING_QUESTION_INDENT - TASK_NUMBER_COLUMN_WIDTH - MIN_WORK_TYPE_COLUMN_WIDTH;
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

    #[test]
    fn test_render_work_type_icon_and_blank_slots_keep_titles_aligned() {
        let now = Utc::now();

        let mut configured = create_test_session("configured");
        configured.updated_at = now;
        configured.status = SessionStatus::Running;
        configured.work_type = Some("demo-skill".to_string());
        configured.label = Some("Configured".to_string());

        let mut unconfigured = create_test_session("unconfigured");
        unconfigured.updated_at = now;
        unconfigured.status = SessionStatus::Running;
        unconfigured.label = Some("Unconfigured".to_string());

        let mut unknown = create_test_session("unknown");
        unknown.updated_at = now;
        unknown.status = SessionStatus::Running;
        unknown.work_type = Some("missing-skill".to_string());
        unknown.label = Some("Unknown".to_string());

        let sessions = vec![configured, unconfigured, unknown];
        let mut agent_config = AgentConfig::default();
        agent_config.work_types.insert(
            "demo-skill".to_string(),
            AgentWorkTypeConfig {
                icon: "\u{e0b1}".to_string(),
                color: AgentWorkTypeColor::Named(
                    crate::shared::config::AgentWorkTypeNamedColor::LightBlue,
                ),
            },
        );
        let output =
            render_to_string_with_agent_config(&sessions, Some(1), now, 80, 12, agent_config);

        let expected = indoc! {"
             agent watch                                    0 needs you · 3 running · 0 idle
             ── RUNNING (3) ────────────────────────────────────────────────────────────────
            >● project               \u{e0b1} Configured                                   just now
             ● project                 Unconfigured                                 just now
             ● project                 Unknown                                      just now






             ?: keys   /: search   Tab: focus   C-b: sidebar   q: quit"};

        assert_eq!(output, expected);
    }

    #[test]
    fn test_render_wide_work_type_icon_keeps_title_and_question_aligned() {
        let now = Utc::now();

        let mut waiting = create_test_session("wide");
        waiting.updated_at = now;
        waiting.status = SessionStatus::WaitingInput;
        waiting.work_type = Some("wide-skill".to_string());
        waiting.label = Some("Wide".to_string());
        waiting.current_tool = Some("Choose".to_string());

        let mut unknown = create_test_session("unknown");
        unknown.updated_at = now;
        unknown.status = SessionStatus::Running;
        unknown.work_type = Some("missing-skill".to_string());
        unknown.label = Some("Blank".to_string());

        let sessions = vec![waiting, unknown];
        let output = render_to_string_with(&sessions, Some(1), now, 80, 12, |app| {
            app.agent_config.work_types.insert(
                "wide-skill".to_string(),
                AgentWorkTypeConfig {
                    icon: "界".to_string(),
                    color: AgentWorkTypeColor::Named(
                        crate::shared::config::AgentWorkTypeNamedColor::Red,
                    ),
                },
            );
        });

        let expected = indoc! {"
             agent watch                                    1 needs you · 1 running · 0 idle
             ── NEEDS YOU ──────────────────────────────────────────────────────────────────
            >◐ project               界  Wide                                        just now
                                        “Choose”

             ── RUNNING (1) ────────────────────────────────────────────────────────────────
             ● project                  Blank                                       just now




             ?: keys   /: search   Tab: focus   C-b: sidebar   q: quit"};

        assert_eq!(output, expected);
    }

    #[test]
    fn test_render_work_type_icon_uses_configured_color() {
        let now = Utc::now();
        let mut session = create_test_session("configured");
        session.updated_at = now;
        session.status = SessionStatus::Running;
        session.work_type = Some("demo-skill".to_string());

        let buffer = render_buffer_with(&[session], Some(1), now, 80, 9, |app| {
            app.agent_config.work_types.insert(
                "demo-skill".to_string(),
                AgentWorkTypeConfig {
                    icon: "\u{e0b1}".to_string(),
                    color: AgentWorkTypeColor::Named(
                        crate::shared::config::AgentWorkTypeNamedColor::LightBlue,
                    ),
                },
            );
        });

        let icon_column = (WAITING_QUESTION_INDENT - MIN_WORK_TYPE_COLUMN_WIDTH) as u16;
        assert_eq!(
            (
                buffer[(icon_column, 2)].symbol(),
                buffer[(icon_column, 2)].fg
            ),
            ("\u{e0b1}", Color::LightBlue)
        );
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
             agent watch                                    0 needs you · 1 running · 0 idle
             ── RUNNING (1) ────────────────────────────────────────────────────────────────
            >● project                 project                                      just now





             ?: keys   /: search   Tab: focus   C-b: sidebar   q: quit"};

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
            is_closed: false,
        }
    }

    #[test]
    fn test_render_task_number_appears_before_breadcrumb_and_title() {
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
             agent watch                                    0 needs you · 1 running · 0 idle
             ── RUNNING (1) ────────────────────────────────────────────────────────────────
            >● project         #42     project                                      just now





             ?: keys   /: search   Tab: focus   C-b: sidebar   q: quit"};

        assert_eq!(output, expected);
    }

    #[test]
    fn test_render_task_kin_highlight_dims_unrelated_task_number() {
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

        let task_number_column = TASK_NUMBER_COLUMN_START as u16;
        assert_ne!(buffer[(task_number_column, 2)].fg, DIM_FG);
        assert_ne!(buffer[(task_number_column, 3)].fg, DIM_FG);
        assert_eq!(buffer[(task_number_column, 4)].fg, DIM_FG);
    }

    #[test]
    fn test_render_closed_task_number_strikes_through_number_only() {
        let now = Utc::now();

        let mut session = create_test_session("s1");
        session.updated_at = now;
        session.status = SessionStatus::Running;

        let sessions = vec![session];
        let buffer = render_buffer_with(&sessions, Some(1), now, 80, 9, |app| {
            app.task_by_session.insert(
                "s1".to_string(),
                SessionTask {
                    is_closed: true,
                    ..linked_task(42, "Fix the bug", None)
                },
            );
        });

        // The number and title positions follow the shared column widths.
        let number_start = TASK_NUMBER_COLUMN_START as u16;
        let title_col = WAITING_QUESTION_INDENT as u16;
        let number_end = number_start + "#42".width() as u16;
        let number_cols = number_start..number_end;
        let padding_cols = number_end..title_col;

        for x in number_cols {
            assert_eq!(buffer[(x, 2)].fg, Color::Indexed(97), "column {x}");
            assert!(
                buffer[(x, 2)].modifier.contains(Modifier::CROSSED_OUT),
                "column {x}"
            );
        }
        for x in padding_cols {
            assert!(
                !buffer[(x, 2)].modifier.contains(Modifier::CROSSED_OUT),
                "column {x}"
            );
        }
        assert!(
            !buffer[(title_col, 2)]
                .modifier
                .contains(Modifier::CROSSED_OUT)
        );
        assert_ne!(buffer[(title_col, 2)].fg, Color::Indexed(97));
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
             agent watch                                    0 needs you · 1 running · 0 idle
             ── RUNNING (1) ────────────────────────────────────────────────────────────────
            >◎ project                 project                                      just now





             ?: keys   /: search   Tab: focus   C-b: sidebar   q: quit"};

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
             agent watch                                    1 needs you · 0 running · 0 idle
             ── NEEDS YOU ──────────────────────────────────────────────────────────────────
            >◐ project                 project                                      just now
                                       “Which approach do you prefer?”





             ?: keys   /: search   Tab: focus   C-b: sidebar   q: quit"};

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
             agent watch                                    1 needs you · 0 running · 0 idle
             ── NEEDS YOU ──────────────────────────────────────────────────────────────────
            >◐ project                 project                                      just now
                                       “”





             ?: keys   /: search   Tab: focus   C-b: sidebar   q: quit"};

        assert_eq!(output, expected);
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
             agent watch                                    0 needs you · 3 running · 0 idle
             ── RUNNING (3) ────────────────────────────────────────────────────────────────
            >● project                 project ▸2                                   just now
             ● project                 project › project                            just now
             ● project                 project › project                            just now




             ?: keys   /: search   Tab: focus   C-b: sidebar   q: quit"};

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
             agent watch                                    1 needs you · 1 running · 0 idle
             ── NEEDS YOU ──────────────────────────────────────────────────────────────────
            >◐ project                 project › project                                  2m
                                       “Pick one”

             ── RUNNING (1) ────────────────────────────────────────────────────────────────
             ● project                 project ▸1                                   just now




             ?: keys   /: search   Tab: focus   C-b: sidebar   q: quit"};

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
             agent watch                                    1 needs you · 1 running · 0 idle
             ── NEEDS YOU ──────────────────────────────────────────────────────────────────
             ◐ project                 project                                      just now
                                       “Pick one”

             ── RUNNING (1) ────────────────────────────────────────────────────────────────
            >● project                 project                                      just now




             ?: keys   /: search   Tab: focus   C-b: sidebar   q: quit"};

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
             agent watch                                    1 needs you · 1 running · 2 idle
             ── NEEDS YOU ──────────────────────────────────────────────────────────────────
             ◐ project                 project                                      just now
                                       “Pick one”

             ── RUNNING (1) ────────────────────────────────────────────────────────────────
             ● project                 project                                      just now

             ── UNREAD (1) ─────────────────────────────────────────────────────────────────
             ✱ project                 project                                      just now

             ── PAUSED (1) ─────────────────────────────────────────────────────────────────
            >⏸ project                 project                                      just now


             ?: keys   /: search   Tab: focus   C-b: sidebar   q: quit"};

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
             agent watch                                    1 needs you · 1 running · 0 idle
             ── NEEDS YOU ──────────────────────────────────────────────────────────────────
             ◐ project                 project                                      just now
                                       “Pick one”

             ── RUNNING (1) ────────────────────────────────────────────────────────────────
            >● project                 project                                      just now




             ?: keys   /: search   Tab: focus   C-b: sidebar   q: quit"};

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
             agent watch                                    0 needs you · 0 running · 2 idle
            ── PAUSED (2) ─────────────────────────────────────────────────────────────────
            ⏸ project                 project                                      just now
            ⏸ project                 project                                      just now





             ?: keys   /: search   Tab: focus   C-b: sidebar   q: quit"};

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
    #[case::beyond_cap_ancestor_a(2, false, Color::Reset)]
    #[case::great_grandparent_b(3, true, Color::Indexed(146))]
    #[case::grandparent_c(4, true, Color::Indexed(111))]
    #[case::parent_d(5, true, Color::Indexed(39))]
    #[case::cursor_row_e(6, true, Color::Reset)]
    fn test_render_kin_highlight_ancestor_ramp_and_cap(
        #[case] row_y: u16,
        #[case] has_breadcrumb: bool,
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
        let breadcrumb_width = if has_breadcrumb {
            "project › ".width()
        } else {
            0
        };
        let title_column = WAITING_QUESTION_INDENT + breadcrumb_width;

        assert_eq!(buffer[(title_column as u16, row_y)].fg, expected_fg);
    }

    #[rstest]
    // `selected` is the cursor. `sibling` shares its immediate parent
    // (distance 1); `cousin` shares its grandparent via a different parent
    // (distance 2); `great_uncle` shares only the great-grandparent root,
    // one generation past `MAX_COLLATERAL_KIN_DISTANCE`, and must render
    // uncolored, same as the cursor row itself. Rows: chrome(y0),
    // header(y1), root(y2) gp_a(y3) gp_b(y4) parent_a(y5) parent_a2(y6)
    // selected(y7) sibling(y8) cousin(y9), in input order.
    #[case::beyond_cap_great_uncle(4, Color::Reset)]
    #[case::cursor_row_selected(7, Color::Reset)]
    #[case::sibling(8, Color::Indexed(129))]
    #[case::cousin(9, Color::Indexed(135))]
    fn test_render_kin_highlight_collateral_ramp_and_cap(
        #[case] row_y: u16,
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
        let title_column = WAITING_QUESTION_INDENT + "project › ".width();

        assert_eq!(buffer[(title_column as u16, row_y)].fg, expected_fg);
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

        // The child's own title starts after its breadcrumb in row 4. The
        // child is a direct descendant of the cursor, but the query "project"
        // matches it too, and search hits must win over kin coloring.
        let title_column = WAITING_QUESTION_INDENT + "project › ".width();
        assert_eq!(buffer[(title_column as u16, 4)].fg, Color::Yellow);
    }
}
