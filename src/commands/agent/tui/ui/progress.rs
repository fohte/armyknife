use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

use crate::commands::agent::tui::{
    app::App,
    progress::{PrProgressRow, ProgressMark, ProgressRow, WorkProgressRow, work_row_count},
};

pub(super) fn render_progress(frame: &mut Frame, area: Rect, app: &mut App) {
    let selected = app.selected_session();
    let title = selected
        .and_then(|session| app.get_cached_title(&session.session_id))
        .or_else(|| selected.map(|session| session.session_id.as_str()))
        .unwrap_or("no session selected");
    let title = match app.progress_error.as_deref() {
        Some(_) => format!("PROGRESS: {title} · PR status unavailable"),
        None => format!("PROGRESS: {title}"),
    };
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(Style::default().fg(if app.progress_focused {
            Color::Cyan
        } else {
            Color::DarkGray
        }))
        .title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(selected) = selected else {
        frame.render_widget(
            Paragraph::new("No session selected").style(Style::default().fg(Color::DarkGray)),
            inner,
        );
        return;
    };
    if app.progress_session_id.as_deref() != Some(selected.session_id.as_str()) {
        frame.render_widget(
            Paragraph::new("Loading delegated progress...")
                .style(Style::default().fg(Color::DarkGray)),
            inner,
        );
        return;
    }

    let work_row_count = work_row_count(&app.progress_rows);
    let work_rows = app
        .progress_rows
        .iter()
        .take(work_row_count)
        .filter_map(|row| match row {
            ProgressRow::Work(work) => Some(work.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    if work_row_count == 0 {
        frame.render_widget(
            Paragraph::new("No progress available").style(Style::default().fg(Color::DarkGray)),
            inner,
        );
        return;
    }

    let has_implementation = app
        .progress_rows
        .iter()
        .any(|row| matches!(row, ProgressRow::ImplementationHeader { .. }));
    let has_pr_rows = app.progress_rows.iter().skip(work_row_count).any(|row| {
        matches!(
            row,
            ProgressRow::PullRequest(_) | ProgressRow::ChecklistPullRequest { .. }
        )
    });
    let plain_pr_row_count = app
        .progress_rows
        .iter()
        .skip(work_row_count)
        .filter(|row| matches!(row, ProgressRow::PullRequest(_)))
        .count();
    let header_height = usize::from(has_pr_rows && !has_implementation) as u16;
    let first_checklist_pr_row = first_checklist_pr_row_index(&app.progress_rows, work_row_count);
    let minimum_data_height = if has_implementation {
        first_checklist_pr_row.map_or(
            usize::from(app.progress_rows.len() > work_row_count),
            |index| index + 1,
        )
    } else {
        usize::from(has_pr_rows).max(plain_pr_row_count.min(3))
    };
    let available_data_height = inner.height.saturating_sub(header_height);
    let reserved_work_height =
        u16::from(work_row_count > 0 && !has_implementation).min(available_data_height);
    let minimum_data_height = (minimum_data_height.min(usize::from(u16::MAX)) as u16)
        .min(available_data_height.saturating_sub(reserved_work_height));
    let work_height = work_row_count.min(usize::from(
        available_data_height.saturating_sub(minimum_data_height),
    )) as u16;
    let [work_area, header_area, rows_area] = Layout::vertical([
        Constraint::Length(work_height),
        Constraint::Length(header_height),
        Constraint::Min(minimum_data_height),
    ])
    .areas(inner);
    render_work_rows(frame, work_area, &work_rows, app);

    if has_implementation {
        render_checklist_rows(frame, rows_area, app, work_row_count);
    } else if has_pr_rows {
        render_stage_header(frame, header_area, inner.width);
        render_pr_rows(frame, rows_area, app);
    } else if let Some(error) = app.progress_error.as_deref() {
        frame.render_widget(
            Paragraph::new(error).style(Style::default().fg(Color::DarkGray)),
            rows_area,
        );
    } else {
        frame.render_widget(
            Paragraph::new("No delegated sessions").style(Style::default().fg(Color::DarkGray)),
            rows_area,
        );
    }
}

fn render_work_rows(frame: &mut Frame, area: Rect, rows: &[WorkProgressRow], app: &App) {
    let visible_count = usize::from(area.height);
    let max_start = rows.len().saturating_sub(visible_count);
    let cursor_work_index = app
        .progress_list_state
        .selected()
        .filter(|selected_index| *selected_index < rows.len());
    let selected_session_index = app.selected_session_id().and_then(|selected_session_id| {
        rows.iter()
            .position(|row| row.session_id == selected_session_id)
    });
    let start = cursor_work_index
        .or(selected_session_index)
        .map(|selected_index| {
            selected_index
                .saturating_sub(visible_count.saturating_sub(1))
                .min(max_start)
        })
        .unwrap_or(0);
    for (visible_index, row) in rows.iter().skip(start).take(visible_count).enumerate() {
        let row_index = start + visible_index;
        let row_area = Rect {
            x: area.x,
            y: area.y.saturating_add(visible_index as u16),
            width: area.width,
            height: 1,
        };
        render_work_row(frame, row_area, row, app, row_index);
    }
}

fn render_work_row(
    frame: &mut Frame,
    area: Rect,
    row: &WorkProgressRow,
    app: &App,
    row_index: usize,
) {
    let selected = app.progress_list_state.selected() == Some(row_index);
    let marker = if selected && app.progress_focused {
        "› "
    } else if selected {
        "> "
    } else {
        "  "
    };
    let mut spans = vec![
        Span::raw(marker),
        Span::styled(row.mark.symbol(), mark_style(row.mark)),
        Span::raw(" "),
        Span::styled(
            row.role.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(label) = row.status_label.as_deref() {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(label.to_string(), mark_style(row.mark)));
    }
    if let Some(note) = row.note.as_deref() {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            note.to_string(),
            Style::default().fg(Color::DarkGray),
        ));
    }
    let background = if selected {
        if app.progress_focused {
            Color::DarkGray
        } else {
            Color::Indexed(236)
        }
    } else {
        Color::Reset
    };
    let row_style = Style::default().bg(background);
    let row_style = if app.selected_session_id() == Some(row.session_id.as_str()) {
        row_style.add_modifier(Modifier::BOLD)
    } else {
        row_style
    };
    frame.render_widget(Paragraph::new(Line::from(spans)).style(row_style), area);
}

fn render_stage_header(frame: &mut Frame, area: Rect, width: u16) {
    let title_width = (width as usize).saturating_sub(24);
    let title_padding = " ".repeat(title_width.min(28));
    let header = Row::new(vec![
        Cell::from(title_padding),
        Cell::from("実装"),
        Cell::from("差分"),
        Cell::from("body"),
        Cell::from("CI"),
        Cell::from("merge"),
    ])
    .style(Style::default().fg(Color::DarkGray));
    let table = Table::new(std::iter::empty::<Row<'_>>(), stage_widths())
        .header(header)
        .column_spacing(1);
    frame.render_widget(table, area);
}

fn render_pr_rows(frame: &mut Frame, area: Rect, app: &mut App) {
    let table_rows = app
        .progress_rows
        .iter()
        .filter_map(|row| match row {
            ProgressRow::PullRequest(row) => Some(pr_table_row(row)),
            ProgressRow::Work(_)
            | ProgressRow::ImplementationHeader { .. }
            | ProgressRow::BatchHeader { .. }
            | ProgressRow::ChecklistPullRequest { .. }
            | ProgressRow::PlannedPullRequest { .. } => None,
        })
        .collect::<Vec<_>>();
    let table = Table::new(table_rows, stage_widths())
        .column_spacing(1)
        .highlight_symbol(if app.progress_focused { "› " } else { "> " })
        .row_highlight_style(Style::default().bg(if app.progress_focused {
            Color::DarkGray
        } else {
            Color::Indexed(236)
        }));
    frame.render_stateful_widget(table, area, &mut app.progress_table_state);
}

fn render_checklist_rows(frame: &mut Frame, area: Rect, app: &mut App, work_row_count: usize) {
    let table_rows = app
        .progress_rows
        .iter()
        .skip(work_row_count)
        .map(checklist_table_row)
        .collect::<Vec<_>>();
    if app.progress_table_state.selected().is_none()
        && app.progress_table_state.offset() == 0
        && area.height > 0
        && let Some(row_index) = first_checklist_pr_row_index(&app.progress_rows, work_row_count)
        && row_index >= usize::from(area.height)
    {
        *app.progress_table_state.offset_mut() = row_index + 1 - usize::from(area.height);
    }
    let table = Table::new(table_rows, stage_widths())
        .column_spacing(1)
        .highlight_symbol(if app.progress_focused { "› " } else { "> " })
        .row_highlight_style(Style::default().bg(if app.progress_focused {
            Color::DarkGray
        } else {
            Color::Indexed(236)
        }));
    frame.render_stateful_widget(table, area, &mut app.progress_table_state);
}

fn first_checklist_pr_row_index(rows: &[ProgressRow], work_row_count: usize) -> Option<usize> {
    rows.iter().skip(work_row_count).position(|row| {
        matches!(
            row,
            ProgressRow::ChecklistPullRequest { .. } | ProgressRow::PlannedPullRequest { .. }
        )
    })
}

fn checklist_table_row(row: &ProgressRow) -> Row<'static> {
    match row {
        ProgressRow::ImplementationHeader {
            merged_pr_count,
            total_pr_count,
            collapsed,
        } => {
            let disclosure = if *collapsed { "▸" } else { "▾" };
            let mut cells = vec![Cell::from(format!(
                "{disclosure} 実装 (PR {merged_pr_count}/{total_pr_count})"
            ))];
            if !collapsed {
                cells.extend([
                    Cell::from("実装"),
                    Cell::from("差分"),
                    Cell::from("body"),
                    Cell::from("CI"),
                    Cell::from("merge"),
                ]);
            } else {
                cells.extend((0..5).map(|_| Cell::from("")));
            }
            Row::new(cells).style(Style::default().fg(Color::DarkGray))
        }
        ProgressRow::BatchHeader {
            title,
            merged_pr_count,
            collapsed,
            ..
        } => {
            let title = if *collapsed {
                format!("  ▸ {title} ({merged_pr_count} PR merged)")
            } else {
                format!("  ▾ {title}")
            };
            Row::new(vec![
                Cell::from(title),
                Cell::from(""),
                Cell::from(""),
                Cell::from(""),
                Cell::from(""),
                Cell::from(""),
            ])
            .style(Style::default().fg(Color::DarkGray))
        }
        ProgressRow::ChecklistPullRequest { row, .. } => pr_table_row_with_indent(row, "    "),
        ProgressRow::PlannedPullRequest { title, .. } => Row::new(vec![
            Cell::from(format!("    ○ {title}")).style(Style::default().fg(Color::DarkGray)),
            Cell::from(""),
            Cell::from(""),
            Cell::from(""),
            Cell::from(""),
            Cell::from(""),
        ]),
        ProgressRow::Work(_) | ProgressRow::PullRequest(_) => Row::new(Vec::<Cell>::new()),
    }
}

fn pr_table_row(row: &PrProgressRow) -> Row<'static> {
    pr_table_row_with_indent(row, "")
}

fn pr_table_row_with_indent(row: &PrProgressRow, indent: &str) -> Row<'static> {
    let mut name = match row.number {
        Some(number) => format!("{indent}#{number} {}", row.title),
        None => format!("{indent}{}", row.title),
    };
    if row.lookup_unavailable {
        name.push_str(" (PR unavailable)");
    }
    Row::new(vec![
        Cell::from(name),
        mark_cell(row.stages.implementation),
        mark_cell(row.stages.diff_review),
        mark_cell(row.stages.pr_body),
        mark_cell(row.stages.ci),
        mark_cell(row.stages.merge),
    ])
}

fn mark_cell(mark: ProgressMark) -> Cell<'static> {
    Cell::from(mark.symbol()).style(mark_style(mark))
}

fn stage_widths() -> [Constraint; 6] {
    [
        Constraint::Min(8),
        Constraint::Length(4),
        Constraint::Length(4),
        Constraint::Length(5),
        Constraint::Length(3),
        Constraint::Length(6),
    ]
}

fn mark_style(mark: ProgressMark) -> Style {
    let color = match mark {
        ProgressMark::Done => Color::Green,
        ProgressMark::User => Color::Yellow,
        ProgressMark::Working => Color::Green,
        ProgressMark::Waiting => Color::Cyan,
        ProgressMark::NotStarted => Color::DarkGray,
    };
    Style::default().fg(color)
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, style::Modifier};
    use rstest::{fixture, rstest};

    use super::*;
    use crate::commands::agent::tui::{
        app::App,
        progress::{ProgressRow, StageMarks},
        ui::test_support::create_test_session,
    };

    #[fixture]
    fn task_work_rows() -> Vec<WorkProgressRow> {
        [
            ("peer-one", "helper"),
            ("peer-two", "helper"),
            ("peer-three", "helper"),
            ("peer-four", "helper"),
            ("selected", "builder"),
        ]
        .into_iter()
        .map(|(session_id, role)| WorkProgressRow {
            session_id: session_id.to_string(),
            role: role.to_string(),
            mark: ProgressMark::Working,
            status_label: None,
            note: None,
            pane_id: None,
        })
        .collect()
    }

    #[fixture]
    fn delegate_pr_row() -> PrProgressRow {
        PrProgressRow {
            session_id: "delegate".to_string(),
            title: "Example task change".to_string(),
            number: Some(17),
            url: Some("https://example.test/pull/17".to_string()),
            pane_id: None,
            is_ended: true,
            lookup_unavailable: false,
            stages: StageMarks {
                implementation: ProgressMark::Done,
                diff_review: ProgressMark::User,
                pr_body: ProgressMark::Working,
                ci: ProgressMark::Waiting,
                merge: ProgressMark::NotStarted,
            },
        }
    }

    fn buffer_snapshot_with_bold(buffer: &Buffer) -> Vec<(String, Vec<u16>)> {
        (0..buffer.area.height)
            .map(|y| {
                let cells = (0..buffer.area.width)
                    .map(|x| &buffer[(x, y)])
                    .collect::<Vec<_>>();
                let text = cells
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string();
                let content_width = cells
                    .iter()
                    .rposition(|cell| !cell.symbol().trim().is_empty())
                    .map_or(0, |column| column + 1);
                let bold_columns = (0..content_width)
                    .filter(|column| cells[*column].modifier.contains(Modifier::BOLD))
                    .map(|column| column as u16)
                    .collect();
                (text, bold_columns)
            })
            .collect()
    }

    fn buffer_snapshot(buffer: &Buffer) -> Vec<String> {
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    fn progress_snapshot(
        terminal: &mut Terminal<TestBackend>,
        app: &mut App,
    ) -> (Vec<String>, usize) {
        terminal
            .draw(|frame| render_progress(frame, frame.area(), app))
            .unwrap();
        (
            buffer_snapshot(terminal.backend().buffer()),
            app.progress_table_state.offset(),
        )
    }

    #[test]
    fn renders_no_delegates_without_a_loading_message() {
        let mut app = App::with_sessions(vec![create_test_session("selected")]);
        app.progress_session_id = Some("selected".to_string());
        app.progress_rows = vec![ProgressRow::Work(WorkProgressRow {
            session_id: "selected".to_string(),
            role: "session".to_string(),
            mark: ProgressMark::Working,
            status_label: None,
            note: None,
            pane_id: None,
        })];

        let backend = TestBackend::new(48, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_progress(frame, frame.area(), &mut app))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let actual = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            actual,
            vec![
                "│PROGRESS: project",
                "│  ● session",
                "│No delegated sessions",
            ],
        );
    }

    #[test]
    fn renders_work_and_delegated_pr_stages() {
        let mut session = create_test_session("selected");
        session.work_type = Some("builder".to_string());
        let mut app = App::with_sessions(vec![session]);
        app.progress_focused = true;
        app.progress_session_id = Some("selected".to_string());
        app.progress_rows = vec![
            ProgressRow::Work(WorkProgressRow {
                session_id: "selected".to_string(),
                role: "builder".to_string(),
                mark: ProgressMark::Working,
                status_label: Some("implementing".to_string()),
                note: None,
                pane_id: None,
            }),
            ProgressRow::PullRequest(PrProgressRow {
                session_id: "delegate".to_string(),
                title: "Sample change".to_string(),
                number: Some(17),
                url: Some("https://example.test/pull/17".to_string()),
                pane_id: None,
                is_ended: true,
                lookup_unavailable: false,
                stages: StageMarks {
                    implementation: ProgressMark::Done,
                    diff_review: ProgressMark::User,
                    pr_body: ProgressMark::Working,
                    ci: ProgressMark::Waiting,
                    merge: ProgressMark::NotStarted,
                },
            }),
            ProgressRow::PullRequest(PrProgressRow {
                session_id: "unavailable-delegate".to_string(),
                title: "Sample delegate".to_string(),
                number: None,
                url: None,
                pane_id: None,
                is_ended: false,
                lookup_unavailable: true,
                stages: StageMarks {
                    implementation: ProgressMark::Working,
                    diff_review: ProgressMark::NotStarted,
                    pr_body: ProgressMark::NotStarted,
                    ci: ProgressMark::NotStarted,
                    merge: ProgressMark::NotStarted,
                },
            }),
        ];
        app.progress_list_state.select(Some(1));
        app.progress_table_state.select(Some(0));

        let backend = TestBackend::new(72, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_progress(frame, frame.area(), &mut app))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let actual = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            actual,
            vec![
                "│PROGRESS: project",
                "│  ● builder  implementing",
                "│                                             実 装  差 分  body  CI  merge",
                "│› #17 Sample change                          ✓    ◐    ●     ◷   ○",
                "│  Sample delegate (PR unavailable)           ●    ○    ○     ○   ○",
                "│",
            ],
        );
    }

    #[test]
    fn renders_multiple_task_work_rows_before_their_delegate_pr() {
        let mut app = App::with_sessions(vec![create_test_session("selected")]);
        app.progress_focused = true;
        app.progress_session_id = Some("selected".to_string());
        app.progress_rows = vec![
            ProgressRow::Work(WorkProgressRow {
                session_id: "selected".to_string(),
                role: "planner".to_string(),
                mark: ProgressMark::Working,
                status_label: None,
                note: None,
                pane_id: None,
            }),
            ProgressRow::Work(WorkProgressRow {
                session_id: "same-task".to_string(),
                role: "builder".to_string(),
                mark: ProgressMark::Working,
                status_label: None,
                note: None,
                pane_id: None,
            }),
            ProgressRow::PullRequest(PrProgressRow {
                session_id: "delegate".to_string(),
                title: "Example task change".to_string(),
                number: Some(17),
                url: Some("https://example.test/pull/17".to_string()),
                pane_id: None,
                is_ended: true,
                lookup_unavailable: false,
                stages: StageMarks {
                    implementation: ProgressMark::Done,
                    diff_review: ProgressMark::User,
                    pr_body: ProgressMark::Working,
                    ci: ProgressMark::Waiting,
                    merge: ProgressMark::NotStarted,
                },
            }),
        ];
        app.progress_list_state.select(Some(1));
        app.progress_table_state.select(None);

        let backend = TestBackend::new(72, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_progress(frame, frame.area(), &mut app))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let actual = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            actual,
            vec![
                "│PROGRESS: project",
                "│  ● planner",
                "│› ● builder",
                "│                                             実 装  差 分  body  CI  merge",
                "│#17 Example task change                      ✓    ◐    ●     ◷   ○",
                "│",
            ],
        );
    }

    #[test]
    fn keeps_the_selected_work_row_visible_when_rows_exceed_the_pane() {
        let mut app = App::with_sessions(vec![create_test_session("selected")]);
        app.progress_focused = true;
        app.progress_session_id = Some("selected".to_string());
        app.progress_rows = ["worker-one", "worker-two", "worker-three", "worker-four"]
            .into_iter()
            .map(|session_id| {
                ProgressRow::Work(WorkProgressRow {
                    session_id: session_id.to_string(),
                    role: session_id.to_string(),
                    mark: ProgressMark::Working,
                    status_label: None,
                    note: None,
                    pane_id: None,
                })
            })
            .collect();
        app.progress_list_state.select(Some(3));

        let backend = TestBackend::new(40, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_progress(frame, frame.area(), &mut app))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let actual = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            actual,
            vec![
                "│PROGRESS: project",
                "│  ● worker-two",
                "│  ● worker-three",
                "│› ● worker-four",
            ],
        );
    }

    #[rstest]
    fn selected_task_work_row_stays_bold_while_a_pr_row_is_selected(
        task_work_rows: Vec<WorkProgressRow>,
        delegate_pr_row: PrProgressRow,
    ) {
        let mut selected_session = create_test_session("selected");
        selected_session.work_type = Some("builder".to_string());
        let mut app = App::with_sessions(vec![selected_session]);
        app.progress_focused = true;
        app.progress_session_id = Some("selected".to_string());
        app.progress_rows = task_work_rows
            .into_iter()
            .map(ProgressRow::Work)
            .chain([ProgressRow::PullRequest(delegate_pr_row)])
            .collect();
        if let ProgressRow::Work(row) = &mut app.progress_rows[4] {
            row.status_label = Some("implementing".to_string());
            row.note = Some("detail".to_string());
        }
        app.progress_list_state.select(Some(5));
        app.progress_table_state.select(Some(0));

        let backend = TestBackend::new(72, 7);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_progress(frame, frame.area(), &mut app))
            .unwrap();

        assert_eq!(
            buffer_snapshot_with_bold(terminal.backend().buffer()),
            vec![
                ("│PROGRESS: project".to_string(), vec![]),
                ("│  ● helper".to_string(), (5..=10).collect()),
                ("│  ● helper".to_string(), (5..=10).collect()),
                ("│  ● helper".to_string(), (5..=10).collect()),
                (
                    "│  ● builder  implementing detail".to_string(),
                    (1..=32).collect(),
                ),
                (
                    "│                                             実 装  差 分  body  CI  merge"
                        .to_string(),
                    vec![],
                ),
                (
                    "│› #17 Example task change                    ✓    ◐    ●     ◷   ○"
                        .to_string(),
                    vec![],
                ),
            ],
        );
    }

    #[rstest]
    #[case::regular_pane(
        7,
        vec![
            "│PROGRESS: project",
            "│  ● helper",
            "│> ● builder",
            "│                                             実 装  差 分  body  CI  merge",
            "│> #17 Change 0                               ✓    ◐    ●     ◷   ○",
            "│  #18 Change 1                               ✓    ◐    ●     ◷   ○",
            "│  #19 Change 2                               ✓    ◐    ●     ◷   ○",
        ],
    )]
    #[case::short_pane(
        5,
        vec![
            "│PROGRESS: project",
            "│> ● builder",
            "│                                             実 装  差 分  body  CI  merge",
            "│> #17 Change 0                               ✓    ◐    ●     ◷   ○",
            "│  #18 Change 1                               ✓    ◐    ●     ◷   ○",
        ],
    )]
    fn keeps_pr_rows_visible_without_hiding_the_selected_work_row(
        #[case] height: u16,
        #[case] expected: Vec<&str>,
        task_work_rows: Vec<WorkProgressRow>,
        delegate_pr_row: PrProgressRow,
    ) {
        let mut app = App::with_sessions(vec![create_test_session("selected")]);
        app.progress_session_id = Some("selected".to_string());
        app.progress_rows = task_work_rows
            .into_iter()
            .map(ProgressRow::Work)
            .chain((0..4).map(|index| {
                let mut row = delegate_pr_row.clone();
                row.session_id = format!("delegate-{index}");
                row.title = format!("Change {index}");
                row.number = Some(17 + index);
                ProgressRow::PullRequest(row)
            }))
            .collect();
        app.progress_list_state.select(Some(4));
        app.progress_table_state.select(Some(0));

        let mut terminal = Terminal::new(TestBackend::new(72, height)).unwrap();
        terminal
            .draw(|frame| render_progress(frame, frame.area(), &mut app))
            .unwrap();

        assert_eq!(
            buffer_snapshot(terminal.backend().buffer()),
            expected.into_iter().map(str::to_string).collect::<Vec<_>>(),
        );
    }

    #[rstest]
    fn keeps_a_checklist_pr_row_visible_when_work_rows_exceed_the_pane(
        task_work_rows: Vec<WorkProgressRow>,
        delegate_pr_row: PrProgressRow,
    ) {
        let mut app = App::with_sessions(vec![create_test_session("selected")]);
        app.progress_session_id = Some("selected".to_string());
        app.progress_rows = task_work_rows
            .into_iter()
            .map(ProgressRow::Work)
            .chain([
                ProgressRow::ImplementationHeader {
                    merged_pr_count: 0,
                    total_pr_count: 1,
                    collapsed: false,
                },
                ProgressRow::BatchHeader {
                    batch_id: "batch-one".to_string(),
                    title: "Batch one".to_string(),
                    merged_pr_count: 0,
                    collapsed: true,
                },
                ProgressRow::BatchHeader {
                    batch_id: "batch-two".to_string(),
                    title: "Batch two".to_string(),
                    merged_pr_count: 0,
                    collapsed: false,
                },
                ProgressRow::ChecklistPullRequest {
                    batch_id: "batch-two".to_string(),
                    row: delegate_pr_row,
                },
            ])
            .collect();
        app.progress_list_state.select(Some(0));

        let backend = TestBackend::new(84, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_progress(frame, frame.area(), &mut app))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let actual = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            actual,
            vec![
                "│PROGRESS: project",
                "│  ▸ Batch one (0 PR merged)",
                "│  ▾ Batch two",
                "│    #17 Example task change                              ✓    ◐    ●     ◷   ○",
            ],
        );
    }

    #[rstest]
    fn preserves_checklist_scroll_offset_when_moving_up(
        task_work_rows: Vec<WorkProgressRow>,
        delegate_pr_row: PrProgressRow,
    ) {
        let mut app = App::with_sessions(vec![create_test_session("selected")]);
        app.progress_focused = true;
        app.progress_session_id = Some("selected".to_string());
        app.progress_rows = task_work_rows
            .into_iter()
            .map(ProgressRow::Work)
            .chain([ProgressRow::ImplementationHeader {
                merged_pr_count: 0,
                total_pr_count: 8,
                collapsed: false,
            }])
            .chain((1..=6).map(|batch_number| ProgressRow::BatchHeader {
                batch_id: format!("batch-{batch_number}"),
                title: format!("Batch {batch_number}"),
                merged_pr_count: 0,
                collapsed: true,
            }))
            .chain((0..8).map(|pr_index| {
                let mut row = delegate_pr_row.clone();
                row.session_id = format!("delegate-{pr_index}");
                row.title = format!("Change {pr_index}");
                row.number = Some(17 + pr_index);
                ProgressRow::ChecklistPullRequest {
                    batch_id: "batch-7".to_string(),
                    row,
                }
            }))
            .collect();

        let work_row_count = work_row_count(&app.progress_rows);
        let last_pr_row_index = app.progress_rows.len() - work_row_count - 1;

        let mut terminal = Terminal::new(TestBackend::new(60, 4)).unwrap();
        let initial = progress_snapshot(&mut terminal, &mut app);
        app.progress_list_state
            .select(Some(work_row_count + last_pr_row_index));
        app.progress_table_state.select(Some(last_pr_row_index));
        let selected_last_pr = progress_snapshot(&mut terminal, &mut app);
        app.select_progress_row(-1);
        let selected_previous_pr = progress_snapshot(&mut terminal, &mut app);
        app.select_progress_row(-1);
        app.select_progress_row(-1);
        app.select_progress_row(-1);
        let scrolled_up = progress_snapshot(&mut terminal, &mut app);

        assert_eq!(
            vec![initial, selected_last_pr, selected_previous_pr, scrolled_up],
            vec![
                (
                    vec![
                        "│PROGRESS: project".to_string(),
                        "│  ▸ Batch 5 (0 PR merged)".to_string(),
                        "│  ▸ Batch 6 (0 PR merged)".to_string(),
                        "│    #17 Change 0                 ✓    ◐    ●     ◷   ○".to_string(),
                    ],
                    5,
                ),
                (
                    vec![
                        "│PROGRESS: project".to_string(),
                        "│      #22 Change 5               ✓    ◐    ●     ◷   ○".to_string(),
                        "│      #23 Change 6               ✓    ◐    ●     ◷   ○".to_string(),
                        "│›     #24 Change 7               ✓    ◐    ●     ◷   ○".to_string(),
                    ],
                    12,
                ),
                (
                    vec![
                        "│PROGRESS: project".to_string(),
                        "│      #22 Change 5               ✓    ◐    ●     ◷   ○".to_string(),
                        "│›     #23 Change 6               ✓    ◐    ●     ◷   ○".to_string(),
                        "│      #24 Change 7               ✓    ◐    ●     ◷   ○".to_string(),
                    ],
                    12,
                ),
                (
                    vec![
                        "│PROGRESS: project".to_string(),
                        "│›     #20 Change 3               ✓    ◐    ●     ◷   ○".to_string(),
                        "│      #21 Change 4               ✓    ◐    ●     ◷   ○".to_string(),
                        "│      #22 Change 5               ✓    ◐    ●     ◷   ○".to_string(),
                    ],
                    10,
                ),
            ],
        );
    }

    #[test]
    fn renders_implementation_batches_and_unstarted_pull_requests() {
        let mut session = create_test_session("selected");
        session.work_type = Some("builder".to_string());
        let mut app = App::with_sessions(vec![session]);
        app.progress_focused = true;
        app.progress_session_id = Some("selected".to_string());
        app.progress_rows = vec![
            ProgressRow::Work(WorkProgressRow {
                session_id: "selected".to_string(),
                role: "builder".to_string(),
                mark: ProgressMark::Working,
                status_label: None,
                note: None,
                pane_id: None,
            }),
            ProgressRow::ImplementationHeader {
                merged_pr_count: 1,
                total_pr_count: 2,
                collapsed: false,
            },
            ProgressRow::BatchHeader {
                batch_id: "batch-1".to_string(),
                title: "Batch 1".to_string(),
                merged_pr_count: 1,
                collapsed: true,
            },
            ProgressRow::BatchHeader {
                batch_id: "batch-2".to_string(),
                title: "Batch 2".to_string(),
                merged_pr_count: 0,
                collapsed: false,
            },
            ProgressRow::ChecklistPullRequest {
                batch_id: "batch-2".to_string(),
                row: PrProgressRow {
                    session_id: "delegate".to_string(),
                    title: "Sample change".to_string(),
                    number: Some(17),
                    url: Some("https://example.test/pull/17".to_string()),
                    pane_id: None,
                    is_ended: true,
                    lookup_unavailable: false,
                    stages: StageMarks {
                        implementation: ProgressMark::Done,
                        diff_review: ProgressMark::User,
                        pr_body: ProgressMark::Working,
                        ci: ProgressMark::Waiting,
                        merge: ProgressMark::NotStarted,
                    },
                },
            },
            ProgressRow::PlannedPullRequest {
                batch_id: "batch-3".to_string(),
                checklist_item_id: "item-3".to_string(),
                title: "Future change".to_string(),
            },
        ];
        app.progress_list_state.select(Some(4));
        app.progress_table_state.select(Some(3));

        let backend = TestBackend::new(84, 7);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_progress(frame, frame.area(), &mut app))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let actual = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            actual,
            vec![
                "│PROGRESS: project",
                "│  ● builder",
                "│  ▾ 実 装  (PR 1/2)                                        実 装  差 分  body  CI  merge",
                "│    ▸ Batch 1 (1 PR merged)",
                "│    ▾ Batch 2",
                "│›     #17 Sample change                                  ✓    ◐    ●     ◷   ○",
                "│      ○ Future change",
            ],
        );
    }
}
