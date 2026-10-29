use chrono::{DateTime, Utc};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    widgets::{List, ListItem, Paragraph},
};

use crate::commands::agent::tui::app::{App, AppMode};
use crate::commands::agent::tui::session_rows::SessionRow;

use super::super::helpers::DIM_FG;
use super::items::{build_header_item, build_session_item};
/// Renders the TASKS session tree. A session linked to a tq task
/// (`app.task_by_session`) gets a fixed-width task-number column, dimmed unless
/// its task is related to the cursor row's task (see
/// [`crate::commands::agent::tui::session_rows::is_related_task`]).
pub(in crate::commands::agent::tui::ui) fn render_session_list(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    now: DateTime<Utc>,
) {
    let no_filtered_sessions = app.filtered_sessions().is_empty();

    if no_filtered_sessions {
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

    let rows = app.build_display_rows();
    // Build list items and owned row ids from the same `rows`, then drop
    // `rows` (which borrows `app`) before mutating app.
    //
    // `items` must stay in exact 1:1 correspondence with `rows` (same
    // length, same index for each entry): `list_state` indices select into
    // `items`, while `row_keys`/`app.row_sessions` are indexed by `rows`
    // position. A separate `ListItem` for the inter-section blank line
    // would desync the two spaces, so the blank line is instead prepended
    // to the following header's own `ListItem`.
    let mut row_keys = Vec::with_capacity(rows.len());
    let mut items: Vec<ListItem> = Vec::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        row_keys.push(row.session_id().map(str::to_owned));

        let item = match row {
            SessionRow::SectionHeader(header) => build_header_item(header, term_width, i > 0),
            SessionRow::Session(entry) => build_session_item(entry, app, now, term_width, &query),
        };
        items.push(item);
    }
    drop(rows);

    let row_heights = items.iter().map(ListItem::height).collect::<Vec<_>>();
    app.update_rendered_rows(&row_keys, &row_heights, area.height as usize);

    let list = List::new(items)
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol(">");

    frame.render_stateful_widget(list, area, &mut app.list_state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::tui::session_rows::SessionTask;
    use crate::commands::agent::tui::ui::helpers::DIM_FG;
    use crate::commands::agent::tui::ui::test_support::{
        create_test_session, render_buffer, render_buffer_with, render_buffer_with_agent_config,
        render_to_string, render_to_string_with, render_to_string_with_agent_config,
    };
    use crate::commands::agent::types::AgentStatus;
    use crate::commands::agent::types::Session;
    use crate::commands::agent::types::SessionStatus;
    use crate::shared::config::{AgentConfig, AgentWorkTypeColor, AgentWorkTypeConfig};
    use indoc::indoc;
    use ratatui::style::Modifier;
    use rstest::{fixture, rstest};
    use unicode_width::UnicodeWidthStr;

    use super::super::items::{
        MIN_WORK_TYPE_COLUMN_WIDTH, TASK_NUMBER_COLUMN_WIDTH, WAITING_QUESTION_BASE_INDENT,
        tree_branch_text,
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

        // Row 0 is the chrome header, row 1 the "TASKS" section
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

        let mut first = create_test_session("first");
        first.status = SessionStatus::Running;
        let mut second = create_test_session("second");
        second.status = SessionStatus::Running;

        let sessions = vec![first, second];
        let buffer = render_buffer(&sessions, Some(2), now, 80, 12);

        // Column 3 is the first character of the repo column for both rows.
        assert_eq!(buffer[(3, 2)].fg, DIM_FG);
        assert_eq!(buffer[(3, 3)].fg, DIM_FG);
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
             ── TASKS ──────────────────────────────────────────────────────────────────────
            >● project               \u{e0b1} Configured                                   just now
             ● project                 Unconfigured                                 just now
             ● project                 Unknown                                      just now






             ?: keys   Tab: pane   Esc: quit"};

        assert_eq!(output, expected);
    }

    #[test]
    fn test_render_vibe_marker_uses_configured_color_and_dims_inherited_rows() {
        let now = Utc::now();

        let mut explicit = create_test_session("explicit");
        explicit.updated_at = now;
        explicit.status = SessionStatus::Running;
        explicit.label = Some("Explicit".to_string());
        explicit.vibe_enabled = true;

        let mut inherited = create_test_session("inherited");
        inherited.updated_at = now;
        inherited.status = SessionStatus::Running;
        inherited.label = Some("Inherited".to_string());
        inherited.ancestor_session_ids = vec!["explicit".to_string()];

        let mut plain = create_test_session("plain");
        plain.updated_at = now;
        plain.status = SessionStatus::Running;
        plain.label = Some("Plain".to_string());

        let mut persisted_ancestor_child = create_test_session("persisted-ancestor-child");
        persisted_ancestor_child.updated_at = now;
        persisted_ancestor_child.status = SessionStatus::Running;
        persisted_ancestor_child.label = Some("Persisted".to_string());
        persisted_ancestor_child.ancestor_session_ids = vec!["stored-ancestor".to_string()];

        let sessions = vec![explicit.clone(), inherited, plain, persisted_ancestor_child];
        let marker_config = AgentWorkTypeConfig {
            icon: "◆".to_string(),
            color: AgentWorkTypeColor::Named(
                crate::shared::config::AgentWorkTypeNamedColor::LightBlue,
            ),
        };
        let agent_config = AgentConfig {
            vibe_marker: Some(marker_config),
            ..AgentConfig::default()
        };
        let configured_buffer =
            render_buffer_with_agent_config(&sessions, Some(1), now, 80, 12, agent_config, |app| {
                app.vibe_enabled_session_ids
                    .insert("stored-ancestor".to_string());
            });

        let mut plain_config_session = explicit;
        plain_config_session.label = Some("NoConfig".to_string());
        let no_config_buffer = render_buffer(&[plain_config_session], Some(1), now, 80, 9);

        let actual = (
            (2..6)
                .map(|row| rendered_row(&configured_buffer, row))
                .collect::<Vec<_>>(),
            (2..6)
                .map(|row| marker_style_on_row(&configured_buffer, row, "◆"))
                .collect::<Vec<_>>(),
            marker_style_on_row(&no_config_buffer, 2, "◆"),
        );
        let expected = (
            vec![
                ">● project                 Explicit◆ ◐ 0/1                              just now"
                    .to_string(),
                " ● project               └─   Inherited◆                                just now"
                    .to_string(),
                " ● project                 Plain                                        just now"
                    .to_string(),
                " ● project                 Persisted◆                                   just now"
                    .to_string(),
            ],
            vec![
                Some(("◆".to_string(), Color::LightBlue, Modifier::empty())),
                Some(("◆".to_string(), Color::LightBlue, Modifier::DIM)),
                None,
                Some(("◆".to_string(), Color::LightBlue, Modifier::DIM)),
            ],
            None,
        );

        assert_eq!(actual, expected);
    }

    fn rendered_row(buffer: &ratatui::buffer::Buffer, row: u16) -> String {
        let mut text = (0..buffer.area.width)
            .map(|column| buffer[(column, row)].symbol())
            .collect::<String>();
        while text.ends_with(' ') {
            text.pop();
        }
        text
    }

    fn marker_style_on_row(
        buffer: &ratatui::buffer::Buffer,
        row: u16,
        marker: &str,
    ) -> Option<(String, Color, Modifier)> {
        (0..buffer.area.width).find_map(|column| {
            let cell = &buffer[(column, row)];
            (cell.symbol() == marker).then(|| (cell.symbol().to_string(), cell.fg, cell.modifier))
        })
    }

    #[test]
    fn test_render_task_turn_label_is_independent_of_work_type_icon_width() {
        let now = Utc::now();

        let mut waiting = create_test_session("wide");
        waiting.updated_at = now;
        waiting.status = SessionStatus::WaitingInput;
        waiting
            .crit_urls
            .push("https://example.com/review".to_string());
        waiting.work_type = Some("wide-skill".to_string());
        waiting.label = Some("Wide".to_string());

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

        let waiting_row = format!(
            ">◐ project               界  Wide crit{}just now",
            " ".repeat(35)
        );
        let expected = [
            " agent watch                                    1 needs you · 1 running · 0 idle",
            " ── TASKS ──────────────────────────────────────────────────────────────────────",
            waiting_row.as_str(),
            " ● project                  Blank                                       just now",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            " ?: keys   Tab: pane   Esc: quit",
        ]
        .join("\n");

        assert_eq!(output, expected);
    }

    #[rstest]
    #[case::configured_work_type(
        Some("sample-flow"),
        Some("sample-flow"),
        "\u{e0b1}",
        Color::LightBlue
    )]
    #[case::fallback_for_missing_work_type(None, None, "\u{e0b1}", Color::LightBlue)]
    #[case::unknown_work_type_keeps_blank(Some("unknown-flow"), None, " ", Color::Reset)]
    fn test_render_work_type_icon_uses_configured_color(
        #[case] work_type: Option<&str>,
        #[case] configured_work_type: Option<&str>,
        #[case] expected_icon: &str,
        #[case] expected_color: Color,
    ) {
        let now = Utc::now();
        let mut session = create_test_session("configured");
        session.updated_at = now;
        session.status = SessionStatus::Running;
        session.work_type = work_type.map(str::to_owned);

        let buffer = render_buffer_with(&[session], Some(1), now, 80, 9, |app| {
            let config = AgentWorkTypeConfig {
                icon: "\u{e0b1}".to_string(),
                color: AgentWorkTypeColor::Named(
                    crate::shared::config::AgentWorkTypeNamedColor::LightBlue,
                ),
            };
            if let Some(work_type) = configured_work_type {
                app.agent_config
                    .work_types
                    .insert(work_type.to_string(), config);
            } else {
                app.agent_config.work_type_fallback = Some(config);
            }
        });

        let icon_column = (WAITING_QUESTION_INDENT - MIN_WORK_TYPE_COLUMN_WIDTH) as u16;
        assert_eq!(
            (
                buffer[(icon_column, 2)].symbol(),
                buffer[(icon_column, 2)].fg
            ),
            (expected_icon, expected_color)
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
             ── TASKS ──────────────────────────────────────────────────────────────────────
            >● project                 project                                      just now





             ?: keys   Tab: pane   Esc: quit"};

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
    fn test_render_task_number_and_task_title_replace_session_name() {
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
             ── TASKS ──────────────────────────────────────────────────────────────────────
            >● project         #42     Fix the bug                                  just now





             ?: keys   Tab: pane   Esc: quit"};

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
        // Row y: chrome(y0), "TASKS" header(y1), cursor(y2),
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
    fn test_render_background_session_shows_waiting_mark() {
        // Persisted `status` is `Stopped` (main loop idle) with a pending
        // background task, so the session is waiting on that task rather
        // than actually running.
        let now = Utc::now();

        let mut session = create_test_session("s1");
        session.updated_at = now;
        session.status = SessionStatus::Stopped;
        session.pending_bg_task_ids.insert("bg-1".to_string());

        let sessions = vec![session];
        let output = render_to_string(&sessions, Some(1), now, 80, 9);

        let expected = indoc! {"
             agent watch                                    0 needs you · 1 running · 0 idle
             ── TASKS ──────────────────────────────────────────────────────────────────────
            >◷ project                 project                                      just now





             ?: keys   Tab: pane   Esc: quit"};

        assert_eq!(output, expected);

        let buffer = render_buffer(&sessions, Some(1), now, 80, 9);
        assert_eq!(buffer[(1, 2)].fg, Color::Yellow);
    }

    #[fixture]
    fn you_user_turn_session() -> Session {
        let mut session = create_test_session("s1");
        session.status = SessionStatus::Stopped;
        session.label = Some("session-name".to_string());
        session.agent_status = Some(AgentStatus {
            kind: "needs_attention".to_string(),
            note: "choose an option".to_string(),
        });
        session
    }

    #[rstest]
    fn test_render_tasks_row_shows_action_label_without_note(mut you_user_turn_session: Session) {
        let now = Utc::now();
        you_user_turn_session.updated_at = now - chrono::Duration::minutes(48);

        let output = render_to_string(&[you_user_turn_session], Some(1), now, 80, 10);

        let action_row = format!(
            ">◐ project                 session-name needs_attention{}48m",
            " ".repeat(22)
        );
        let expected = [
            " agent watch                                    1 needs you · 0 running · 0 idle",
            " ── TASKS ──────────────────────────────────────────────────────────────────────",
            action_row.as_str(),
            "",
            "",
            "",
            "",
            "",
            "",
            " ?: keys   Tab: pane   Esc: quit",
        ]
        .join("\n");

        assert_eq!(output, expected);
    }

    #[rstest]
    fn test_render_action_label_uses_red_background(you_user_turn_session: Session) {
        let now = Utc::now();
        let buffer = render_buffer(&[you_user_turn_session], None, now, 80, 9);
        let red_background_cells = (0..buffer.area.width)
            .filter_map(|x| {
                let cell = &buffer[(x, 2)];
                (cell.bg == Color::Red).then(|| cell.symbol().to_string())
            })
            .collect::<Vec<_>>();

        let expected = [
            " ", "n", "e", "e", "d", "s", "_", "a", "t", "t", "e", "n", "t", "i", "o", "n", " ",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
        assert_eq!(red_background_cells, expected);
    }

    #[rstest]
    #[case::tight_crit(36, ">◐ pr... a-long-ses...◆crit just now", false)]
    #[case::narrow_agent_status(43, ">◐ project      a-long-sess...◆you just now", true)]
    fn test_narrow_action_label_keeps_session_name_and_time(
        #[case] term_width: u16,
        #[case] expected_row: &str,
        #[case] is_agent_status: bool,
    ) {
        let now = Utc::now();
        let mut root = create_test_session("root");
        root.updated_at = now;
        root.label = Some("a-long-session-name".to_string());
        if is_agent_status {
            root.status = SessionStatus::Stopped;
            root.agent_status = Some(AgentStatus {
                kind: "needs_attention".to_string(),
                note: "choose an option".to_string(),
            });
        } else {
            root.status = SessionStatus::WaitingInput;
            root.crit_urls
                .push("https://example.test/review".to_string());
        }
        root.vibe_enabled = true;

        let mut child = create_test_session("child");
        child.updated_at = now;
        child.ancestor_session_ids = vec!["root".to_string()];

        let agent_config = AgentConfig {
            vibe_marker: Some(AgentWorkTypeConfig {
                icon: "◆".to_string(),
                color: AgentWorkTypeColor::Named(
                    crate::shared::config::AgentWorkTypeNamedColor::LightBlue,
                ),
            }),
            ..AgentConfig::default()
        };
        let buffer = render_buffer_with_agent_config(
            &[root, child],
            Some(1),
            now,
            term_width,
            6,
            agent_config,
            |_| {},
        );

        assert_eq!(rendered_row(&buffer, 2), expected_row);
    }

    #[test]
    fn test_render_failed_user_turn_shows_failure_mark_on_tasks_row() {
        let now = Utc::now();
        let mut failed = create_test_session("failed");
        failed.updated_at = now;
        failed.status = SessionStatus::Failed;
        failed.label = Some("session-name".to_string());

        let output = render_to_string(&[failed], Some(1), now, 80, 9);

        let failed_row = format!(
            ">✗ project                 session-name failed{}just now",
            " ".repeat(26)
        );
        let expected = [
            " agent watch                                    1 needs you · 0 running · 0 idle",
            " ── TASKS ──────────────────────────────────────────────────────────────────────",
            failed_row.as_str(),
            "",
            "",
            "",
            "",
            "",
            " ?: keys   Tab: pane   Esc: quit",
        ]
        .join("\n");

        assert_eq!(output, expected);
    }

    #[test]
    fn test_render_root_with_children_shows_branches_and_user_turn_badge() {
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
             ── TASKS ──────────────────────────────────────────────────────────────────────
            >● project                 project ◐ 0/2                                just now
             ● project               ├─   project                                   just now
             ● project               └─   project                                   just now




             ?: keys   Tab: pane   Esc: quit"};

        assert_eq!(output, expected);
    }

    #[test]
    fn test_render_child_with_different_status_stays_under_parent() {
        let now = Utc::now();

        let mut parent = create_test_session("parent");
        parent.updated_at = now;
        parent.status = SessionStatus::Running;

        let mut child = create_test_session("child");
        child.updated_at = now - chrono::Duration::minutes(2);
        child.ancestor_session_ids = vec!["parent".to_string()];
        child.status = SessionStatus::Paused;

        let sessions = vec![parent, child];
        let output = render_to_string(&sessions, Some(2), now, 80, 12);

        let expected = indoc! {"
             agent watch                                    0 needs you · 1 running · 1 idle
             ── TASKS ──────────────────────────────────────────────────────────────────────
             ● project                 project ◐ 0/1                                just now
            >⏸ project               └─   project                                         2m







             ?: keys   Tab: pane   Esc: quit"};

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
        waiting
            .crit_urls
            .push("https://example.com/review".to_string());

        let mut running = create_test_session("running");
        running.updated_at = now;
        running.status = SessionStatus::Running;

        (now, vec![waiting, running])
    }

    fn expected_waiting_task_row(selected: bool) -> String {
        format!(
            "{}◐ project                 project crit{}just now",
            if selected { ">" } else { " " },
            " ".repeat(33)
        )
    }

    #[rstest]
    fn test_select_next_moves_from_priority_task_to_next_session(
        waiting_and_running_sessions: (DateTime<Utc>, Vec<Session>),
    ) {
        let (now, sessions) = waiting_and_running_sessions;
        let output = render_to_string_with(&sessions, Some(1), now, 80, 12, |app| {
            app.select_next();
        });

        let waiting_row = expected_waiting_task_row(false);
        let expected = [
            " agent watch                                    1 needs you · 1 running · 0 idle",
            " ── TASKS ──────────────────────────────────────────────────────────────────────",
            waiting_row.as_str(),
            ">● project                 project                                      just now",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            " ?: keys   Tab: pane   Esc: quit",
        ]
        .join("\n");

        assert_eq!(output, expected);
    }

    #[rstest]
    fn test_select_next_through_every_tasks_row_lands_on_session_row(
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
        // Walk from the highest-priority session through the last task row.
        let output = render_to_string_with(&sessions, Some(1), now, 80, 16, |app| {
            app.select_next();
            app.select_next();
            app.select_next();
        });

        let waiting_row = expected_waiting_task_row(false);
        let expected = [
            " agent watch                                    1 needs you · 1 running · 2 idle",
            " ── TASKS ──────────────────────────────────────────────────────────────────────",
            waiting_row.as_str(),
            " ● project                 project                                      just now",
            " ✱ project                 project                                      just now",
            ">⏸ project                 project                                      just now",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            " ?: keys   Tab: pane   Esc: quit",
        ]
        .join("\n");

        assert_eq!(output, expected);
    }

    #[rstest]
    fn test_select_previous_wraps_backward_across_section_lands_on_session_row(
        waiting_and_running_sessions: (DateTime<Utc>, Vec<Session>),
    ) {
        let (now, sessions) = waiting_and_running_sessions;
        // Starting on the highest-priority session and going backward wraps
        // to the last session, skipping the section header.
        let output = render_to_string_with(&sessions, Some(1), now, 80, 12, |app| {
            app.select_previous();
        });

        let waiting_row = expected_waiting_task_row(false);
        let expected = [
            " agent watch                                    1 needs you · 1 running · 0 idle",
            " ── TASKS ──────────────────────────────────────────────────────────────────────",
            waiting_row.as_str(),
            ">● project                 project                                      just now",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            " ?: keys   Tab: pane   Esc: quit",
        ]
        .join("\n");

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
    #[case::beyond_cap_ancestor_a(2, None, Color::Reset)]
    #[case::great_grandparent_b(3, Some(vec![]), Color::Indexed(146))]
    #[case::grandparent_c(4, Some(vec![true]), Color::Indexed(111))]
    #[case::parent_d(5, Some(vec![true, true]), Color::Indexed(39))]
    #[case::cursor_row_e(6, Some(vec![true, true, true]), Color::Reset)]
    fn test_render_kin_highlight_ancestor_ramp_and_cap(
        #[case] row_y: u16,
        #[case] tree_prefix: Option<Vec<bool>>,
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
        let title_column =
            WAITING_QUESTION_INDENT + tree_branch_text(tree_prefix.as_deref(), false).width();

        assert_eq!(buffer[(title_column as u16, row_y)].fg, expected_fg);
    }

    #[rstest]
    // `selected` is the cursor. `sibling` shares its immediate parent
    // (distance 1); `cousin` shares its grandparent via a different parent
    // (distance 2); `great_uncle` shares only the great-grandparent root,
    // one generation past `MAX_COLLATERAL_KIN_DISTANCE`, and must render
    // uncolored, same as the cursor row itself. Rows: chrome(y0),
    // header(y1), root(y2), gp_a(y3), parent_a(y4), selected(y5),
    // sibling(y6), parent_a2(y7), cousin(y8), gp_b(y9).
    #[case::beyond_cap_great_uncle(9, Some(vec![]), Color::Reset)]
    #[case::cursor_row_selected(5, Some(vec![false, false]), Color::Reset)]
    #[case::sibling(6, Some(vec![false, false]), Color::Indexed(129))]
    #[case::cousin(8, Some(vec![false, true]), Color::Indexed(135))]
    fn test_render_kin_highlight_collateral_ramp_and_cap(
        #[case] row_y: u16,
        #[case] tree_prefix: Option<Vec<bool>>,
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
        // list_state index: header=0, root=1, gp_a=2, parent_a=3,
        // selected=4, sibling=5, parent_a2=6, cousin=7, gp_b=8.
        let buffer = render_buffer(&sessions, Some(4), now, 80, 20);
        let title_column =
            WAITING_QUESTION_INDENT + tree_branch_text(tree_prefix.as_deref(), false).width();

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

        // The child's own title starts after its tree branch in row 4. The
        // child is a direct descendant of the cursor, but the query "project"
        // matches it too, and search hits must win over kin coloring.
        let title_column = WAITING_QUESTION_INDENT + "├─ ".width();
        assert_eq!(buffer[(title_column as u16, 4)].fg, Color::Yellow);
    }

    #[test]
    fn keeps_multiline_selection_on_the_same_screen_line_after_reordering() {
        use ratatui::{Terminal, backend::TestBackend};
        use std::time::{Duration, Instant};

        let base = Utc::now() - chrono::Duration::minutes(5);
        let mut parent_a = create_test_session("parent-a");
        parent_a.updated_at = base;
        let mut question = create_test_session("question");
        question.status = SessionStatus::WaitingInput;
        question.last_message = Some("permission needed".to_string());
        question.updated_at = base + chrono::Duration::seconds(2);
        question.ancestor_session_ids = vec!["parent-a".to_string()];
        let mut parent_b = create_test_session("parent-b");
        parent_b.updated_at = base + chrono::Duration::seconds(3);
        let mut app = App::with_sessions(vec![parent_a, question, parent_b]);
        let selected = app
            .position_of_session_row("question")
            .expect("question row is displayed");
        app.list_state.select(Some(selected));

        let mut terminal = Terminal::new(TestBackend::new(60, 5)).expect("test terminal created");
        terminal
            .draw(|frame| render_session_list(frame, frame.area(), &mut app, Utc::now()))
            .expect("initial list rendered");
        let initial_top = (0..5).find(|&y| terminal.backend().buffer()[(0, y)].symbol() == ">");

        let start = Instant::now();
        app.note_session_list_interaction_at(start);
        let mut new_child = create_test_session("new-child");
        new_child.updated_at = base + chrono::Duration::seconds(4);
        new_child.ancestor_session_ids = vec!["parent-a".to_string()];
        app.apply_session_store_changes(crate::commands::agent::store::SessionStoreChanges {
            changed_sessions: vec![new_child],
            existing_session_ids: std::collections::HashSet::from([
                "parent-a".to_string(),
                "parent-b".to_string(),
                "question".to_string(),
                "new-child".to_string(),
            ]),
            revision: 1,
        })
        .expect("session-store changes should apply");
        let applied = app.finish_navigation_hold_if_due(start + Duration::from_secs(2));

        terminal
            .draw(|frame| render_session_list(frame, frame.area(), &mut app, Utc::now()))
            .expect("reordered list rendered");
        let reordered_top = (0..5).find(|&y| terminal.backend().buffer()[(0, y)].symbol() == ">");

        assert_eq!(
            (initial_top, applied, reordered_top),
            (Some(3), true, Some(3))
        );
    }

    #[test]
    fn keeps_the_viewport_line_when_search_removes_the_selected_session() {
        use ratatui::{Terminal, backend::TestBackend};
        use std::path::PathBuf;

        let now = Utc::now();
        let sessions = [
            ("keep-before", true, 6),
            ("drop-before", false, 5),
            ("drop-target", false, 4),
            ("keep-after-1", true, 3),
            ("keep-after-2", true, 2),
            ("keep-after-3", true, 1),
        ]
        .map(|(id, matches, seconds)| {
            let mut session = create_test_session(id);
            session.cwd = PathBuf::from(format!("/home/user/{id}"));
            session.updated_at = now - chrono::Duration::seconds(seconds);
            if matches {
                session.label = Some(format!("keep {id}"));
            }
            session
        });
        let mut app = App::with_sessions(sessions.to_vec());
        let target = app
            .position_of_session_row("drop-target")
            .expect("target row is displayed");
        app.list_state.select(Some(target));

        let mut terminal = Terminal::new(TestBackend::new(60, 4)).expect("test terminal created");
        terminal
            .draw(|frame| render_session_list(frame, frame.area(), &mut app, now))
            .expect("initial list rendered");
        let initial_top = (0..4).find(|&y| terminal.backend().buffer()[(0, y)].symbol() == ">");

        app.enter_search_mode();
        app.update_search_query("keep-before".to_string());
        app.update_search_query("keep".to_string());
        let selected_after_filter = app
            .selected_session()
            .map(|session| session.session_id.clone());
        terminal
            .draw(|frame| render_session_list(frame, frame.area(), &mut app, now))
            .expect("filtered list rendered");
        let filtered_top = (0..4).find(|&y| terminal.backend().buffer()[(0, y)].symbol() == ">");

        assert_eq!(
            (initial_top, selected_after_filter, filtered_top),
            (Some(3), Some("keep-before".to_string()), Some(3)),
        );
    }
}
