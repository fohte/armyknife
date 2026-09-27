use chrono::{DateTime, Utc};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use crate::commands::agent::tui::{
    app::App,
    tq_sidebar::{SidebarRow, SidebarRowKind},
};
use crate::commands::agent::types::DisplayStatus;

use super::helpers::{DIM_FG, status_color, truncate};

pub(super) const SIDEBAR_PERCENTAGE: u16 = 40;
pub(super) const MIN_SESSION_LIST_WIDTH: u16 = 48;

pub(super) fn minimum_total_width() -> u16 {
    MIN_SESSION_LIST_WIDTH * 100 / (100 - SIDEBAR_PERCENTAGE)
}

pub(super) fn render_tq_sidebar(frame: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [list_area, footer_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);

    let rows = app.sidebar_rows();
    app.sync_sidebar_list_state(&rows);

    let items = rows
        .iter()
        .map(|row| build_row_item(row, inner.width as usize))
        .collect::<Vec<_>>();
    let list = List::new(items)
        .highlight_style(Style::default().bg(if app.sidebar_focused {
            Color::DarkGray
        } else {
            Color::Indexed(236)
        }))
        .highlight_symbol(if app.sidebar_focused { ">" } else { " " });
    frame.render_stateful_widget(list, list_area, &mut app.sidebar_list_state);

    if let Some((message, style)) = footer_message(app, Utc::now()) {
        frame.render_widget(Paragraph::new(Line::styled(message, style)), footer_area);
    }
}

fn build_row_item(row: &SidebarRow, area_width: usize) -> ListItem<'static> {
    if row.kind == SidebarRowKind::Separator {
        return ListItem::new(Line::styled(
            "─".repeat(area_width.saturating_sub(1)),
            Style::default().fg(DIM_FG),
        ));
    }

    let counts = row
        .session_counts
        .as_ref()
        .map(|counts| format_counts(counts))
        .unwrap_or_default();
    let count_width = counts
        .iter()
        .map(|(text, _)| text.width() + 1)
        .sum::<usize>();
    let content_width = area_width.saturating_sub(1);
    let left_width = content_width.saturating_sub(count_width);
    let spans = row_label_spans(row, left_width);
    let left_width_used = spans
        .iter()
        .map(|span| span.content.as_ref().width())
        .sum::<usize>();
    let padding = left_width.saturating_sub(left_width_used);
    let mut line_spans = spans;
    line_spans.push(Span::raw(" ".repeat(padding)));
    for (text, style) in counts {
        line_spans.push(Span::styled(text, style));
        line_spans.push(Span::raw(" "));
    }
    ListItem::new(Line::from(line_spans))
}

fn row_label_spans(row: &SidebarRow, width: usize) -> Vec<Span<'static>> {
    match row.kind {
        SidebarRowKind::All => vec![Span::styled(
            "◉ すべて",
            Style::default().add_modifier(Modifier::BOLD),
        )],
        SidebarRowKind::Untasked => vec![Span::styled("○ タスクなし", Style::default().fg(DIM_FG))],
        SidebarRowKind::Project => project_spans(row, width),
        SidebarRowKind::Task => task_spans(row, width),
        SidebarRowKind::Separator => Vec::new(),
    }
}

fn project_spans(row: &SidebarRow, width: usize) -> Vec<Span<'static>> {
    let toggle = if row.has_children {
        if row.expanded { "▾ " } else { "▸ " }
    } else {
        "  "
    };
    let title = format!("▣ {}", row.title);
    let title_width = width.saturating_sub(toggle.width());
    let title = truncate(&title, title_width);
    let mut style = row
        .project
        .as_ref()
        .and_then(|project| project.color.as_deref())
        .and_then(parse_project_color)
        .map(|color| Style::default().fg(color))
        .unwrap_or_default();
    if row
        .project
        .as_ref()
        .and_then(|project| project.status.as_deref())
        .is_some_and(|status| status != "active")
    {
        style = style.add_modifier(Modifier::DIM);
    }
    vec![
        Span::raw(toggle.to_string()),
        Span::styled(title, style.add_modifier(Modifier::BOLD)),
    ]
}

fn task_spans(row: &SidebarRow, width: usize) -> Vec<Span<'static>> {
    let Some(task) = row.task.as_ref() else {
        return Vec::new();
    };
    let indent = "  ".repeat(row.depth);
    let toggle = if row.has_children {
        if row.expanded { "▾ " } else { "▸ " }
    } else {
        "  "
    };
    let strike = task.status == crate::infra::tq::TqTaskStatus::Completed;
    let exceptional_completion = task
        .status_reason
        .as_deref()
        .is_some_and(|reason| matches!(reason, "not_planned" | "duplicate"));
    let status_style = if exceptional_completion {
        Style::default()
            .fg(DIM_FG)
            .add_modifier(Modifier::CROSSED_OUT)
    } else if strike {
        Style::default()
            .fg(Color::Indexed(141))
            .add_modifier(Modifier::CROSSED_OUT)
    } else {
        Style::default()
    };
    let marker = if strike && !exceptional_completion {
        "✓ "
    } else {
        ""
    };
    let title = format!("#{} {}", task.number, task.title);
    let today = Utc::now().date_naive().to_string();
    let due_overdue = task.status != crate::infra::tq::TqTaskStatus::Completed
        && task
            .due_date
            .as_deref()
            .is_some_and(|date| date < today.as_str());
    let commitment = task.commitment.as_deref().unwrap_or_default();
    let tags = match (due_overdue, commitment) {
        (true, "") => " overdue".to_string(),
        (true, value) => format!(" overdue {value}"),
        (false, "") => String::new(),
        (false, value) => format!(" {value}"),
    };
    let fixed_width = indent.width() + toggle.width() + marker.width();
    let tag_width = tags.width();
    let title = truncate(&title, width.saturating_sub(fixed_width + tag_width));
    let mut spans = vec![Span::raw(indent), Span::raw(toggle.to_string())];
    if !marker.is_empty() {
        spans.push(Span::styled(
            marker,
            Style::default().fg(if exceptional_completion {
                DIM_FG
            } else {
                Color::Indexed(141)
            }),
        ));
    }
    spans.push(Span::styled(title, status_style));
    if !tags.is_empty() {
        spans.push(Span::styled(tags, tag_style(due_overdue, commitment)));
    }
    spans
}

fn tag_style(overdue: bool, commitment: &str) -> Style {
    if overdue {
        Style::default().fg(Color::Red)
    } else {
        match commitment {
            "inbox" => Style::default().fg(Color::Yellow),
            "active" => Style::default().fg(Color::Green),
            "someday" => Style::default().fg(Color::DarkGray),
            _ => Style::default().fg(DIM_FG),
        }
    }
}

fn format_counts(counts: &[(DisplayStatus, usize)]) -> Vec<(String, Style)> {
    counts
        .iter()
        .map(|(status, count)| {
            (
                format!("{}{}", status.display_symbol(), count),
                Style::default().fg(status_color(*status)),
            )
        })
        .collect()
}

fn parse_project_color(value: &str) -> Option<Color> {
    let hex = value.strip_prefix('#')?;
    if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        6 => Some(Color::Rgb(
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
        )),
        3 => Some(Color::Rgb(
            u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?,
            u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?,
            u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?,
        )),
        _ => None,
    }
}

fn footer_message(app: &App, now: DateTime<Utc>) -> Option<(String, Style)> {
    if app.tq_refreshing {
        return Some(("↻ refreshing".to_string(), Style::default().fg(DIM_FG)));
    }
    if !app.tq_refresh_failed {
        return None;
    }
    match app.tq_snapshot.as_ref() {
        Some(snapshot) => {
            let minutes = now
                .signed_duration_since(snapshot.fetched_at)
                .num_minutes()
                .max(0);
            Some((
                format!("! tq: {minutes}m old   r: retry"),
                Style::default().fg(Color::Yellow),
            ))
        }
        None => Some((
            "! tq load failed   r: retry".to_string(),
            Style::default().fg(Color::Red),
        )),
    }
}
