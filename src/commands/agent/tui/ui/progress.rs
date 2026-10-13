use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

use crate::commands::agent::tui::{
    app::App,
    progress::{PrProgressRow, ProgressMark, ProgressRow, WorkProgressRow},
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

    let Some(ProgressRow::Work(work)) = app.progress_rows.first().cloned() else {
        frame.render_widget(
            Paragraph::new("No progress available").style(Style::default().fg(Color::DarkGray)),
            inner,
        );
        return;
    };

    let has_delegates = app.progress_rows.len() > 1;
    let work_height = 1;
    let header_height = if has_delegates { 1 } else { 0 };
    let [work_area, header_area, rows_area] = Layout::vertical([
        Constraint::Length(work_height),
        Constraint::Length(header_height),
        Constraint::Min(0),
    ])
    .areas(inner);
    render_work_row(frame, work_area, &work, app);

    if has_delegates {
        render_stage_header(frame, header_area, inner.width);
        render_pr_rows(frame, rows_area, app);
    } else if app.progress_loading {
        frame.render_widget(
            Paragraph::new("Loading delegated PRs...").style(Style::default().fg(Color::DarkGray)),
            rows_area,
        );
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

fn render_work_row(frame: &mut Frame, area: Rect, row: &WorkProgressRow, app: &App) {
    let selected = app.progress_list_state.selected() == Some(0);
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
            Style::default().add_modifier(ratatui::style::Modifier::BOLD),
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
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(background)),
        area,
    );
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
            ProgressRow::Work(_) => None,
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

fn pr_table_row(row: &PrProgressRow) -> Row<'static> {
    let name = match row.number {
        Some(number) => format!("#{number} {}", row.title),
        None => row.title.clone(),
    };
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
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::commands::agent::tui::{
        app::App,
        progress::{ProgressRow, StageMarks},
        ui::test_support::create_test_session,
    };

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
        app.progress_table_state.select(Some(0));

        let backend = TestBackend::new(72, 5);
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
                "│",
            ],
        );
    }
}
