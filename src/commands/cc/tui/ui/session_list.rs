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
use crate::commands::cc::tui::session_tree::{
    TreeEntry, build_line1_tree_prefix, build_line2_tree_prefix, build_parent_child_connector,
    build_separator_tree_prefix, build_session_tree,
};
use crate::commands::cc::tui::worktree_session_children::format_relative_time;

use super::helpers::{
    get_session_info, get_title_display_name_fallback, highlight_matches, repo_label_color,
    status_color, time_ago_color, truncate,
};

/// Minimum width for session info on line 1
const MIN_SESSION_INFO_WIDTH: usize = 20;
/// Minimum width for tool/message content on line 2
const MIN_CONTENT_WIDTH: usize = 20;
/// Fixed width for time suffix: "  XXm ago" = ~12 chars
const LINE1_SUFFIX_WIDTH: usize = 12;

/// Renders the session list with tree view.
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

    // Build tree structure from sessions
    let tree_entries = build_session_tree(&filtered_sessions);

    // Collect tree-ordered session IDs and build list items, then drop
    // tree_entries (which borrows filtered_sessions/app) before mutating app.
    let mut tree_session_ids: Vec<String> = Vec::with_capacity(tree_entries.len());
    let items: Vec<ListItem> = tree_entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            tree_session_ids.push(entry.session.session_id.clone());
            let next_entry = tree_entries.get(i + 1);
            let cached_title = app.get_cached_title(&entry.session.session_id);
            let (repo_name, worktree_name) = app
                .get_cached_worktree_labels(&entry.session.cwd)
                .unwrap_or(("", ""));
            create_tree_session_item(
                entry,
                next_entry,
                cached_title,
                now,
                term_width,
                &query,
                repo_name,
                worktree_name,
            )
        })
        .collect();
    drop(tree_entries);
    drop(filtered_sessions);

    // Sync tree-ordered indices so selection maps to the correct session
    let tree_id_refs: Vec<&str> = tree_session_ids.iter().map(|s| s.as_str()).collect();
    app.update_tree_order(&tree_id_refs);

    let list = List::new(items)
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol(">");

    frame.render_stateful_widget(list, area, &mut app.list_state);
}

/// Creates a list item for a session within a tree view.
///
/// Each session renders as 2 lines:
/// - Line 1: [tree_prefix] status_symbol ▎ session_info  label  time_ago
/// - Line 2: [tree_prefix_continuation]  ▎ current_tool or last_message
///
/// Plus separator lines between tree entries (empty lines with connectors).
#[expect(
    clippy::too_many_arguments,
    reason = "Tree-rendering needs entry + neighbour + caches + repo/worktree labels in one call"
)]
fn create_tree_session_item(
    entry: &TreeEntry,
    next_entry: Option<&TreeEntry>,
    cached_title: Option<&str>,
    now: DateTime<Utc>,
    term_width: usize,
    query: &str,
    repo_name: &str,
    worktree_name: &str,
) -> ListItem<'static> {
    let session = entry.session;
    let status_symbol = session.display_symbol();
    let s_color = status_color(session.status);
    let session_info = get_session_info(session, repo_name, worktree_name);
    let label = cached_title
        .map(String::from)
        .unwrap_or_else(|| get_title_display_name_fallback(session));
    let time_ago = format_relative_time(session.updated_at, now);

    let repo_color = repo_label_color(repo_name);
    let bar = Span::styled("▎", Style::default().fg(repo_color));
    let dim_style = Style::default().fg(Color::DarkGray);

    let line1_tree_prefix = build_line1_tree_prefix(entry);
    let line2_tree_prefix = build_line2_tree_prefix(entry);

    // Display width of the tree prefix + status symbol + bar + spacing
    // Line 1: "{tree_prefix}{status} ▎ {session_info}  {label}  {time_ago}"
    let line1_prefix_display_width =
        line1_tree_prefix.width() + status_symbol.width() + " ▎ ".width();
    let line1_fixed_width = line1_prefix_display_width + LINE1_SUFFIX_WIDTH;

    let session_info_width = if term_width > line1_fixed_width + MIN_SESSION_INFO_WIDTH {
        term_width - line1_fixed_width
    } else {
        MIN_SESSION_INFO_WIDTH
    };

    // Line 2: "{tree_prefix}  ▎ {content}"
    let line2_prefix_display_width = line2_tree_prefix.width() + "  ▎ ".width();
    let content_width = if term_width > line2_prefix_display_width + MIN_CONTENT_WIDTH {
        term_width - line2_prefix_display_width
    } else {
        MIN_CONTENT_WIDTH
    };

    let time_ago_fg = time_ago_color(session.updated_at, now);

    // Build combined info string: "session_info  label"
    // Skip label if it duplicates session_info (common when no explicit label is set)
    let combined_info = if label.is_empty() || label == session_info {
        session_info.clone()
    } else {
        format!("{}  {}", session_info, label)
    };
    let truncated_info = truncate(&combined_info, session_info_width);
    let is_paused = session.status == SessionStatus::Paused;
    let paused_style = Style::default().fg(Color::Indexed(245));
    let info_style = if is_paused {
        paused_style
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    };

    // Line 1: [tree_prefix] status ▎ session_info  label  time_ago
    let mut line1_spans = Vec::new();
    if !line1_tree_prefix.is_empty() {
        line1_spans.push(Span::styled(line1_tree_prefix, dim_style));
    }
    line1_spans.push(Span::styled(status_symbol, Style::default().fg(s_color)));
    line1_spans.push(Span::raw(" "));
    line1_spans.push(bar.clone());
    line1_spans.push(Span::raw(" "));
    line1_spans.extend(highlight_matches(&truncated_info, query, info_style));
    line1_spans.push(Span::raw("  "));
    let time_style = if is_paused {
        paused_style
    } else {
        Style::default().fg(time_ago_fg)
    };
    line1_spans.push(Span::styled(time_ago, time_style));
    let line1 = Line::from(line1_spans);

    // Line 2: [tree_prefix_continuation]  ▎ current_tool or last_message
    let line2_content = session
        .current_tool
        .as_deref()
        .or(session.last_message.as_deref())
        .unwrap_or("");
    let truncated_content = truncate(line2_content, content_width);
    let content_style = if is_paused {
        paused_style
    } else {
        Style::default().add_modifier(Modifier::DIM)
    };
    let mut line2_spans = Vec::new();
    if !line2_tree_prefix.is_empty() {
        line2_spans.push(Span::styled(line2_tree_prefix, dim_style));
    }
    line2_spans.push(Span::raw("  "));
    line2_spans.push(bar.clone());
    line2_spans.push(Span::raw(" "));
    line2_spans.extend(highlight_matches(&truncated_content, query, content_style));
    let line2 = Line::from(line2_spans);

    // Build separator lines
    let mut lines = vec![line1, line2];

    // Add separator line(s) between this entry and the next
    if let Some(next) = next_entry {
        if entry.has_children {
            // This node has children, next is its first child:
            // show "│" connector between parent and children block
            let connector = build_parent_child_connector(entry);
            if !connector.is_empty() {
                lines.push(Line::from(Span::styled(connector, dim_style)));
            } else {
                lines.push(Line::from(""));
            }
        } else if next.depth < entry.depth {
            // Going back up the tree: show blank line
            lines.push(Line::from(""));
        } else if next.depth == entry.depth && next.depth == 0 {
            // Between root-level tree groups: blank line separator
            lines.push(Line::from(""));
        } else {
            // Between siblings: show separator with pipe
            let sep = build_separator_tree_prefix(entry);
            if !sep.is_empty() {
                lines.push(Line::from(Span::styled(sep, dim_style)));
            } else {
                lines.push(Line::from(""));
            }
        }
    }
    // No separator after the very last entry

    ListItem::new(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::tui::ui::test_support::{create_test_session, render_to_string};
    use crate::commands::cc::types::TmuxInfo;
    use chrono::Duration;
    use indoc::indoc;
    use rstest::rstest;
    use std::path::PathBuf;

    #[rstest]
    #[case::just_now(0, "just now")]
    #[case::one_minute(60, "1m ago")]
    #[case::two_hours(7200, "2h ago")]
    #[case::one_day(86400, "1d ago")]
    fn test_format_relative_time(#[case] seconds_ago: i64, #[case] expected: &str) {
        let now = Utc::now();
        let dt = now - Duration::seconds(seconds_ago);
        assert_eq!(format_relative_time(dt, now), expected);
    }

    // =========================================================================
    // Full-screen integration tests using TestBackend (tree view)
    // =========================================================================

    #[test]
    fn test_render_flat_sessions_tree_view() {
        let now = Utc::now();

        let mut session1 = create_test_session("s1");
        session1.updated_at = now;
        session1.tmux_info = Some(TmuxInfo {
            session_name: "webapp".to_string(),
            window_name: "dev".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        });
        session1.status = SessionStatus::Running;

        let mut session2 = create_test_session("s2");
        session2.updated_at = now - Duration::minutes(5);
        session2.tmux_info = Some(TmuxInfo {
            session_name: "api".to_string(),
            window_name: "test".to_string(),
            window_index: 1,
            pane_id: "%1".to_string(),
        });
        session2.status = SessionStatus::WaitingInput;

        let sessions = vec![session1, session2];
        let output = render_to_string(&sessions, Some(0), now, 80, 13);

        let expected = indoc! {"
             cc watch                                       1 needs you · 1 running · 0 idle
            >● ▎ project  just now
               ▎

             ◐ ▎ project  5m ago
               ▎






             ?: keys   /: search   Tab: worktree   q: quit"};

        assert_eq!(output, expected);
    }

    #[test]
    fn test_render_full_screen_empty_sessions() {
        let now = Utc::now();
        let sessions: Vec<Session> = vec![];
        let output = render_to_string(&sessions, None, now, 80, 9);

        // Not `indoc!`: every non-blank line here has a leading space (the
        // header/help text itself starts with one), so there is no
        // zero-indent line for `indoc!` to anchor its dedent on.
        let expected = " cc watch                                       0 needs you · 0 running · 0 idle\n  No active Claude Code sessions.\n\n\n\n\n\n\n ?: keys   /: search   Tab: worktree   q: quit";

        assert_eq!(output, expected);
    }

    #[test]
    fn test_render_session_with_last_message() {
        let now = Utc::now();

        let mut session = create_test_session("s1");
        session.updated_at = now;
        session.tmux_info = Some(TmuxInfo {
            session_name: "webapp".to_string(),
            window_name: "dev".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        });
        session.status = SessionStatus::Running;
        session.last_message = Some("I've updated the code as requested.".to_string());

        let sessions = vec![session];
        let output = render_to_string(&sessions, Some(0), now, 80, 9);

        let expected = indoc! {"
             cc watch                                       0 needs you · 1 running · 0 idle
            >● ▎ project  just now
               ▎ I've updated the code as requested.





             ?: keys   /: search   Tab: worktree   q: quit"};

        assert_eq!(output, expected);
    }

    #[test]
    fn test_render_session_without_last_message() {
        let now = Utc::now();

        let mut session = create_test_session("s1");
        session.updated_at = now;
        session.tmux_info = Some(TmuxInfo {
            session_name: "webapp".to_string(),
            window_name: "dev".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        });
        session.status = SessionStatus::Running;

        let sessions = vec![session];
        let output = render_to_string(&sessions, Some(0), now, 80, 9);

        let expected = indoc! {"
             cc watch                                       0 needs you · 1 running · 0 idle
            >● ▎ project  just now
               ▎





             ?: keys   /: search   Tab: worktree   q: quit"};

        assert_eq!(output, expected);
    }

    #[test]
    fn test_render_session_with_label_different_from_info() {
        let now = Utc::now();

        let mut session = create_test_session("s1");
        session.updated_at = now;
        session.cwd = PathBuf::from("/home/user/docs");
        session.status = SessionStatus::Stopped;
        // No tmux, so session_info = "/home/user/docs", fallback title = "docs"
        // These differ, so label "docs" should appear

        let sessions = vec![session];
        let output = render_to_string(&sessions, Some(0), now, 80, 9);

        let expected = indoc! {"
             cc watch                                       0 needs you · 0 running · 1 idle
            >✱ ▎ docs  just now
               ▎





             ?: keys   /: search   Tab: worktree   q: quit"};

        assert_eq!(output, expected);
    }

    // =========================================================================
    // Rendered tree output tests
    // =========================================================================

    #[test]
    fn test_render_parent_child_tree() {
        let now = Utc::now();

        let mut parent = create_test_session("parent");
        parent.updated_at = now;
        parent.tmux_info = Some(TmuxInfo {
            session_name: "app".to_string(),
            window_name: "main".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        });
        parent.status = SessionStatus::Running;
        parent.current_tool = Some("Bash(cargo build)".to_string());

        let mut child = create_test_session("child");
        child.updated_at = now - Duration::minutes(2);
        child.ancestor_session_ids = vec!["parent".to_string()];
        child.tmux_info = Some(TmuxInfo {
            session_name: "app".to_string(),
            window_name: "test".to_string(),
            window_index: 1,
            pane_id: "%1".to_string(),
        });
        child.status = SessionStatus::Running;
        child.current_tool = Some("Bash(cargo test)".to_string());

        let sessions = vec![parent, child];
        let output = render_to_string(&sessions, Some(0), now, 80, 13);

        let expected = indoc! {"
             cc watch                                       0 needs you · 2 running · 0 idle
            >● ▎ project  just now
               ▎ Bash(cargo build)
             │
             └── ● ▎ project  2m ago
                   ▎ Bash(cargo test)






             ?: keys   /: search   Tab: worktree   q: quit"};

        assert_eq!(output, expected);
    }

    #[test]
    fn test_render_multiple_children_tree() {
        let now = Utc::now();

        let mut parent = create_test_session("parent");
        parent.updated_at = now;
        parent.tmux_info = Some(TmuxInfo {
            session_name: "app".to_string(),
            window_name: "main".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        });

        let mut child1 = create_test_session("child1");
        child1.updated_at = now - Duration::minutes(1);
        child1.ancestor_session_ids = vec!["parent".to_string()];
        child1.tmux_info = Some(TmuxInfo {
            session_name: "app".to_string(),
            window_name: "test".to_string(),
            window_index: 1,
            pane_id: "%1".to_string(),
        });

        let mut child2 = create_test_session("child2");
        child2.updated_at = now - Duration::minutes(3);
        child2.ancestor_session_ids = vec!["parent".to_string()];
        child2.tmux_info = Some(TmuxInfo {
            session_name: "app".to_string(),
            window_name: "review".to_string(),
            window_index: 2,
            pane_id: "%2".to_string(),
        });
        child2.status = SessionStatus::WaitingInput;

        let sessions = vec![parent, child1, child2];
        let output = render_to_string(&sessions, Some(0), now, 80, 16);

        let expected = indoc! {"
             cc watch                                       1 needs you · 2 running · 0 idle
            >● ▎ project  just now
               ▎
             │
             ├── ● ▎ project  1m ago
             │     ▎
             │
             └── ◐ ▎ project  3m ago
                   ▎






             ?: keys   /: search   Tab: worktree   q: quit"};

        assert_eq!(output, expected);
    }
}
