use chrono::{DateTime, Utc};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use crate::commands::cc::tui::app::App;
use crate::commands::cc::tui::clean_view::{
    CleanListEntry, CleanLoadState, CleanSection, PrFetchStatus,
};
use crate::commands::cc::tui::worktree_session_children::create_session_child_list_item;

use super::helpers::truncate;

/// Renders the clean view: To delete / Kept sections, repo group
/// headers under each section, one row per worktree, then nested
/// session rows under each worktree.
pub(super) fn render_clean_list(frame: &mut Frame, area: Rect, app: &mut App, now: DateTime<Utc>) {
    let term_width = area.width as usize;
    let state = app.clean_view.state.clone();
    match state {
        CleanLoadState::LoadingPr => {
            // No initial worktree snapshot yet — the only thing to show
            // is the pending state. Once the snapshot arrives the view
            // re-renders as Ready.
            let p = Paragraph::new("  Loading worktrees...")
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(p, area);
            return;
        }
        CleanLoadState::Failed(err) => {
            let line = Line::from(vec![
                Span::styled(
                    "  Failed to load PR status: ",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::styled(err, Style::default().fg(Color::Red)),
            ]);
            frame.render_widget(Paragraph::new(line), area);
            return;
        }
        CleanLoadState::Ready(_) => {}
    }

    let entries = app.clean_view.list_entries();
    if entries.is_empty() {
        let p =
            Paragraph::new("  No worktrees to clean.").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(p, area);
        return;
    }

    // Reserve one line at the top for the PR-fetch status banner when
    // the fetch is either still running or has failed.
    let banner = pr_fetch_banner(&app.clean_view.pr_fetch);
    let (banner_area, list_area) = if banner.is_some() {
        let chunks = Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(area);
        (Some(chunks[0]), chunks[1])
    } else {
        (None, area)
    };
    if let (Some(area), Some(p)) = (banner_area, banner) {
        frame.render_widget(p, area);
    }

    let items: Vec<ListItem> = entries
        .iter()
        .map(|e| create_clean_list_item(e, term_width, now))
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol(">");

    frame.render_stateful_widget(list, list_area, &mut app.clean_view.list_state);
}

fn pr_fetch_banner(status: &PrFetchStatus) -> Option<Paragraph<'static>> {
    match status {
        PrFetchStatus::Loading => Some(
            Paragraph::new("  Fetching PR status... (toggle disabled)")
                .style(Style::default().fg(Color::DarkGray)),
        ),
        PrFetchStatus::Failed(err) => {
            let line = Line::from(vec![
                Span::styled(
                    "  PR fetch failed: ",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::styled(err.clone(), Style::default().fg(Color::Red)),
            ]);
            Some(Paragraph::new(line))
        }
        PrFetchStatus::Done => None,
    }
}

fn create_clean_list_item(
    entry: &CleanListEntry,
    term_width: usize,
    now: DateTime<Utc>,
) -> ListItem<'static> {
    let dim_style = Style::default().fg(Color::DarkGray);
    let bold = Style::default().add_modifier(Modifier::BOLD);

    match entry {
        CleanListEntry::Session(child) => create_session_child_list_item(child, now),
        CleanListEntry::SectionHeader { section, count } => {
            let label = match section {
                CleanSection::ToDelete => format!("── To delete ({count}) "),
                CleanSection::Kept => format!("── Kept ({count}) "),
            };
            let color = match section {
                CleanSection::ToDelete => Color::Red,
                CleanSection::Kept => Color::Green,
            };
            // Pad with em-dashes to fill the row visually.
            let pad_width = term_width.saturating_sub(label.width()).min(80);
            let mut padded = label.clone();
            for _ in 0..pad_width {
                padded.push('─');
            }
            let line = Line::from(vec![Span::styled(
                padded,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            )]);
            ListItem::new(vec![line])
        }
        CleanListEntry::RepoHeader(name) => {
            let line = Line::from(vec![Span::styled(
                format!("▼ {name}"),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )]);
            ListItem::new(vec![line])
        }
        CleanListEntry::Row(row) => {
            let (symbol, color) = if row.has_active {
                ("◐", Color::Yellow)
            } else if row.session_count > 0 {
                ("●", Color::Green)
            } else {
                ("◌", Color::DarkGray)
            };
            let bar = Span::styled("▎", Style::default().fg(Color::DarkGray));

            let primary = if row.repo == row.name || row.branch.is_empty() {
                row.repo.clone()
            } else {
                format!("{} {}", row.repo, row.branch)
            };
            let label = format!("[{}]", row.status_label);
            let primary_truncated =
                truncate(&primary, term_width.saturating_sub(8 + label.width() + 2));

            let line1 = Line::from(vec![
                Span::raw("  "),
                Span::styled(symbol.to_string(), Style::default().fg(color)),
                Span::raw(" "),
                bar.clone(),
                Span::raw(" "),
                Span::styled(primary_truncated, bold),
                Span::raw("  "),
                Span::styled(label, dim_style),
            ]);

            let detail = format!(
                "{} session{} · {}",
                row.session_count,
                if row.session_count == 1 { "" } else { "s" },
                row.path.display()
            );
            let detail = truncate(&detail, term_width.saturating_sub(8));
            let line2 = Line::from(vec![
                Span::raw("    "),
                bar,
                Span::raw(" "),
                Span::styled(detail, dim_style),
            ]);

            ListItem::new(vec![line1, line2])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::tui::ui::test_support::{render_to_string_with, wt_row};

    #[test]
    fn test_render_clean_view_emits_both_section_headers() {
        // Both section headers should render even when one section is
        // empty (e.g. all rows default to Kept while PR fetch loads).
        let now = Utc::now();
        let output = render_to_string_with(&[], None, now, 80, 16, |app| {
            app.set_worktrees(vec![wt_row(
                "armyknife",
                "feat/a",
                "feat-a",
                "/tmp/armyknife/.worktrees/feat-a",
            )]);
            app.enter_clean_view();
        });
        assert!(
            output.contains("To delete") && output.contains("Kept"),
            "expected both section headers, got:\n{output}",
        );
    }
}
