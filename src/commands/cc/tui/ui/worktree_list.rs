use crate::commands::cc::tui::app::App;
use crate::commands::cc::tui::worktree_session_children::create_session_child_list_item;
use crate::commands::cc::tui::worktree_view::{
    WorktreeListEntry, WorktreeLoadState, WorktreeStatus,
};
use chrono::{DateTime, Utc};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph},
};

use super::helpers::{repo_label_color, truncate};

/// Renders the worktree list, grouped by repo.
pub(super) fn render_worktree_list(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    now: DateTime<Utc>,
) {
    let term_width = area.width as usize;
    let state = app.worktree_view.state.clone();
    match state {
        WorktreeLoadState::Loading => {
            let p = Paragraph::new("  Loading worktrees...")
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(p, area);
            return;
        }
        WorktreeLoadState::Failed(err) => {
            let line = Line::from(vec![
                Span::styled(
                    "  Failed to load worktrees: ",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::styled(err, Style::default().fg(Color::Red)),
            ]);
            frame.render_widget(Paragraph::new(line), area);
            return;
        }
        WorktreeLoadState::Loaded(_) => {}
    }

    let entries = app.worktree_view.list_entries();
    if entries.is_empty() {
        let p = Paragraph::new("  No linked worktrees discovered.")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(p, area);
        return;
    }

    let items: Vec<ListItem> = entries
        .iter()
        .map(|e| create_worktree_list_item(e, term_width, now))
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol(">");

    frame.render_stateful_widget(list, area, &mut app.worktree_view.list_state);
}

/// Returns the symbol + color used for one worktree row.
fn worktree_status_glyph(status: WorktreeStatus) -> (&'static str, Color) {
    match status {
        WorktreeStatus::Orphan => ("◌", Color::DarkGray),
        WorktreeStatus::Active => ("◐", Color::Yellow),
        WorktreeStatus::Idle => ("●", Color::Green),
    }
}

fn create_worktree_list_item(
    entry: &WorktreeListEntry,
    term_width: usize,
    now: DateTime<Utc>,
) -> ListItem<'static> {
    let dim_style = Style::default().fg(Color::DarkGray);
    let bold = Style::default().add_modifier(Modifier::BOLD);

    match entry {
        WorktreeListEntry::Session(child) => create_session_child_list_item(child, now),
        WorktreeListEntry::RepoHeader(name) => {
            let line = Line::from(vec![Span::styled(
                format!("▼ {}", name),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )]);
            ListItem::new(vec![line])
        }
        WorktreeListEntry::Worktree(row) => {
            let (symbol, color) = worktree_status_glyph(row.status());
            let repo_color = repo_label_color(&row.repo);
            let bar = Span::styled("▎", Style::default().fg(repo_color));

            // Line 1: "  {status} ▎ {repo} {branch}"
            let primary = if row.repo == row.name || row.branch.is_empty() {
                row.repo.clone()
            } else {
                format!("{} {}", row.repo, row.branch)
            };
            let primary = truncate(&primary, term_width.saturating_sub(8));
            let line1 = Line::from(vec![
                Span::raw("  "),
                Span::styled(symbol.to_string(), Style::default().fg(color)),
                Span::raw(" "),
                bar.clone(),
                Span::raw(" "),
                Span::styled(primary, bold),
            ]);

            // Line 2: "    ▎ {n} sessions · {path}"
            // Leading width = indent (2) + status glyph width (1) + space (1) = 4
            // so the bar lines up under the bar on line 1.
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
    use crate::commands::cc::tui::app::View;
    use crate::commands::cc::tui::ui::test_support::{render_to_string_with, wt_row};
    use indoc::indoc;

    #[test]
    fn test_render_worktree_view_grouped_rows_snapshot() {
        // Locks in row layout: header title, repo group, status glyph,
        // and crucially the column alignment between line 1 (header +
        // bar) and line 2 (bar + detail). If the bar drifts between
        // lines, the snapshot diff makes it obvious.
        let now = Utc::now();
        let output = render_to_string_with(&[], None, now, 80, 12, |app| {
            app.view = View::Worktree;
            app.set_worktrees(vec![
                wt_row(
                    "armyknife",
                    "feat/a",
                    "feat-a",
                    "/tmp/armyknife/.worktrees/feat-a",
                ),
                wt_row("specs", "main", "main", "/tmp/specs/.worktrees/main"),
            ]);
        });

        let expected = indoc! {"
             cc watch                                       0 needs you · 0 running · 0 idle
             ▼ armyknife
            >  ◌ ▎ armyknife feat/a
                 ▎ 0 sessions · /tmp/armyknife/.worktrees/feat-a
             ▼ specs
               ◌ ▎ specs main
                 ▎ 0 sessions · /tmp/specs/.worktrees/main




             ?: keys   Enter/f: focus   Tab: switch view   q: quit"};

        assert_eq!(output, expected);
    }

    #[test]
    fn test_render_worktree_view_loading_snapshot() {
        let now = Utc::now();
        let output = render_to_string_with(&[], None, now, 80, 9, |app| {
            app.view = View::Worktree;
            // No set_worktrees call → state stays Loading.
        });

        // Not `indoc!`: every non-blank line here has a leading space (the
        // header/help text itself starts with one), so there is no
        // zero-indent line for `indoc!` to anchor its dedent on.
        let expected = " cc watch                                       0 needs you · 0 running · 0 idle\n  Loading worktrees...\n\n\n\n\n\n\n ?: keys   Enter/f: focus   Tab: switch view   q: quit";

        assert_eq!(output, expected);
    }
}
