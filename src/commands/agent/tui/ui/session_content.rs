use chrono::{DateTime, Utc};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::commands::agent::tui::app::{App, View};
use crate::commands::agent::tui::tq_sidebar::SidebarSelection;

use super::clean_list::render_clean_list;
use super::session_list::render_session_list;
use super::tq_sidebar::{
    MIN_SESSION_LIST_WIDTH, SIDEBAR_WIDTH, minimum_total_width, render_tq_sidebar,
};

pub(super) fn render_main_list(frame: &mut Frame, area: Rect, app: &mut App, now: DateTime<Utc>) {
    match app.view {
        View::Session => {
            let sidebar_available = area.width >= minimum_total_width();
            app.set_sidebar_available(sidebar_available);
            if app.sidebar_visible && sidebar_available {
                let [sidebar_area, session_area] = Layout::horizontal([
                    Constraint::Length(SIDEBAR_WIDTH),
                    Constraint::Min(MIN_SESSION_LIST_WIDTH),
                ])
                .areas(area);
                render_tq_sidebar(frame, sidebar_area, app);
                render_session_list(frame, session_area, app, now);
            } else {
                render_session_list(frame, area, app, now);
            }
        }
        View::Clean => render_clean_list(frame, area, app, now),
    }
}

pub(super) fn render_sidebar_scope(frame: &mut Frame, area: Rect, app: &App) {
    let (label, detail) = match &app.sidebar_selection {
        SidebarSelection::Task(task_id) => {
            let Some(snapshot) = app.tq_snapshot.as_ref() else {
                return;
            };
            let Some(task) = snapshot.tasks.get(task_id) else {
                return;
            };
            let label = format!("  #{} {}", task.number, task.title);
            let today = Utc::now().date_naive().to_string();
            let due = task.due_date.as_deref().map(|date| {
                if task.status != crate::infra::tq::TqTaskStatus::Completed && date < today.as_str()
                {
                    "overdue".to_string()
                } else {
                    format!("due {date}")
                }
            });
            (
                label,
                [task.commitment.clone(), due]
                    .into_iter()
                    .flatten()
                    .collect(),
            )
        }
        SidebarSelection::Project(project_id) => {
            let Some(snapshot) = app.tq_snapshot.as_ref() else {
                return;
            };
            let Some(project) = snapshot.projects.get(project_id) else {
                return;
            };
            (
                format!("  ▣ {}", project.title),
                project.status.clone().into_iter().collect(),
            )
        }
        SidebarSelection::Untasked => ("  ○ タスクなし".to_string(), Vec::new()),
        SidebarSelection::All => return,
    };
    let mut spans = vec![Span::styled(
        label,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )];
    for item in detail {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(item, Style::default().fg(Color::DarkGray)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
