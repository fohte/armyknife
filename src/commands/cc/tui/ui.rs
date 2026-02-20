use crate::commands::cc::types::{Session, SessionStatus};
use chrono::{DateTime, Utc};
#[cfg(test)]
use ratatui::widgets::ListState;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};
use std::collections::{HashMap, HashSet};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::app::{App, AppMode};

/// Minimum width for session info on line 1
const MIN_SESSION_INFO_WIDTH: usize = 20;
/// Minimum width for tool/message content on line 2
const MIN_CONTENT_WIDTH: usize = 20;
/// Fixed width for time suffix: "  XXm ago" = ~12 chars
const LINE1_SUFFIX_WIDTH: usize = 12;

/// Number of distinct hue slots for repo label colors.
/// Using a prime number helps avoid systematic collisions with
/// common string patterns.
const REPO_LABEL_HUE_SLOTS: u64 = 31;

// Green-to-gray gradient for the "time ago" label
const TIME_AGO_BRIGHT_GREEN: Color = Color::Indexed(82);
const TIME_AGO_GREEN: Color = Color::Indexed(78);
const TIME_AGO_DARK_GREEN: Color = Color::Indexed(72);
const TIME_AGO_DARKER_GREEN: Color = Color::Indexed(65);
const TIME_AGO_LIGHT_GRAY: Color = Color::Indexed(245);
const TIME_AGO_DARK_GRAY: Color = Color::Indexed(241);

// =========================================================================
// Tree view data structures and building logic
// =========================================================================

/// Tracks the connector type inherited from each ancestor level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TreePrefixSegment {
    /// "│   " - ancestor at this depth is NOT the last child
    Pipe,
    /// "    " - ancestor at this depth IS the last child
    Space,
}

/// A tree node wrapping a session with tree layout metadata.
#[derive(Debug)]
struct TreeEntry<'a> {
    session: &'a Session,
    /// Depth in the tree (0 = root)
    depth: usize,
    /// Whether this node is the last child of its parent
    is_last_child: bool,
    /// Prefix segments inherited from ancestors (length = depth)
    prefix_segments: Vec<TreePrefixSegment>,
    /// Whether this node has any children
    has_children: bool,
}

/// Builds a tree structure from a flat list of sessions.
///
/// Sessions are organized into trees using `ancestor_session_ids`.
/// Each session finds its nearest living ancestor among the displayed sessions.
/// Sessions without parents (or whose parents are not in the list) become roots.
fn build_session_tree<'a>(sessions: &[&'a Session]) -> Vec<TreeEntry<'a>> {
    if sessions.is_empty() {
        return Vec::new();
    }

    // Build a set of session IDs that are currently displayed
    let displayed_ids: HashSet<&str> = sessions.iter().map(|s| s.session_id.as_str()).collect();

    // For each session, find its parent (nearest living ancestor) among displayed sessions
    let mut parent_map: HashMap<&str, &str> = HashMap::new();
    for session in sessions {
        if let Some(parent_id) = find_nearest_living_ancestor(session, &displayed_ids) {
            parent_map.insert(session.session_id.as_str(), parent_id);
        }
    }

    // Build children map: parent_id -> Vec<child session>
    let mut children_map: HashMap<&str, Vec<&Session>> = HashMap::new();
    let mut root_sessions: Vec<&Session> = Vec::new();

    for &session in sessions {
        if let Some(&parent_id) = parent_map.get(session.session_id.as_str()) {
            children_map.entry(parent_id).or_default().push(session);
        } else {
            root_sessions.push(session);
        }
    }

    // Build set of sessions that have children
    let has_children: HashSet<&str> = children_map.keys().copied().collect();

    // DFS to flatten tree into ordered entries
    let mut entries = Vec::new();

    for (root_idx, root_session) in root_sessions.iter().enumerate() {
        let is_last_root = root_idx == root_sessions.len() - 1;
        build_tree_entries_dfs(
            root_session,
            0,
            is_last_root,
            &[],
            &children_map,
            &has_children,
            &mut entries,
        );
    }

    entries
}

/// Recursively builds tree entries via depth-first traversal.
fn build_tree_entries_dfs<'a>(
    session: &'a Session,
    depth: usize,
    is_last_child: bool,
    parent_prefix: &[TreePrefixSegment],
    children_map: &HashMap<&str, Vec<&'a Session>>,
    has_children_set: &HashSet<&str>,
    entries: &mut Vec<TreeEntry<'a>>,
) {
    let has_children = has_children_set.contains(session.session_id.as_str());

    entries.push(TreeEntry {
        session,
        depth,
        is_last_child,
        prefix_segments: parent_prefix.to_vec(),
        has_children,
    });

    if let Some(children) = children_map.get(session.session_id.as_str()) {
        // Build new prefix for children: append segment for current node
        let mut child_prefix = parent_prefix.to_vec();
        if depth > 0 {
            // Non-root nodes contribute a connector to their children's prefix
            if is_last_child {
                child_prefix.push(TreePrefixSegment::Space);
            } else {
                child_prefix.push(TreePrefixSegment::Pipe);
            }
        }

        for (i, child) in children.iter().enumerate() {
            let is_last = i == children.len() - 1;
            build_tree_entries_dfs(
                child,
                depth + 1,
                is_last,
                &child_prefix,
                children_map,
                has_children_set,
                entries,
            );
        }
    }
}

/// Finds the nearest living ancestor of a session among the displayed sessions.
/// Walks `ancestor_session_ids` from the end (nearest ancestor) to the start (root).
fn find_nearest_living_ancestor<'a>(
    session: &'a Session,
    displayed_ids: &HashSet<&str>,
) -> Option<&'a str> {
    // Walk from nearest ancestor to root
    for ancestor_id in session.ancestor_session_ids.iter().rev() {
        if displayed_ids.contains(ancestor_id.as_str()) {
            return Some(ancestor_id.as_str());
        }
    }
    None
}

/// Builds the tree connector prefix string for the first line of a node.
///
/// For root nodes (depth=0): no prefix (empty string)
/// For children: inherited prefix + own connector ("├── " or "└── ")
fn build_line1_tree_prefix(entry: &TreeEntry) -> String {
    if entry.depth == 0 {
        return String::new();
    }

    let mut prefix = String::new();
    for segment in &entry.prefix_segments {
        match segment {
            TreePrefixSegment::Pipe => prefix.push_str("│   "),
            TreePrefixSegment::Space => prefix.push_str("    "),
        }
    }

    if entry.is_last_child {
        prefix.push_str("└── ");
    } else {
        prefix.push_str("├── ");
    }

    prefix
}

/// Builds the tree connector prefix string for the second line (continuation).
///
/// For root nodes: no prefix
/// For children: inherited prefix + continuation ("│   " if not last, "    " if last)
fn build_line2_tree_prefix(entry: &TreeEntry) -> String {
    if entry.depth == 0 {
        return String::new();
    }

    let mut prefix = String::new();
    for segment in &entry.prefix_segments {
        match segment {
            TreePrefixSegment::Pipe => prefix.push_str("│   "),
            TreePrefixSegment::Space => prefix.push_str("    "),
        }
    }

    if entry.is_last_child {
        prefix.push_str("    ");
    } else {
        prefix.push_str("│   ");
    }

    prefix
}

/// Builds the tree connector prefix for separator lines between siblings.
///
/// For root-level separators: "│" below parent
/// For child-level separators: inherited prefix + "│"
fn build_separator_tree_prefix(entry: &TreeEntry) -> String {
    if entry.depth == 0 {
        // Root sessions that have children: show "│" below them
        if entry.has_children {
            return "│".to_string();
        }
        return String::new();
    }

    let mut prefix = String::new();
    for segment in &entry.prefix_segments {
        match segment {
            TreePrefixSegment::Pipe => prefix.push_str("│   "),
            TreePrefixSegment::Space => prefix.push_str("    "),
        }
    }

    // After the last child's separator, only show pipe if not last
    if !entry.is_last_child {
        prefix.push('│');
    }

    prefix
}

/// Builds the tree connector prefix for lines between a parent and its children.
///
/// Shows "│" at the parent's depth level to connect parent to children block.
fn build_parent_child_connector(entry: &TreeEntry) -> String {
    if entry.depth == 0 {
        return "│".to_string();
    }

    let mut prefix = String::new();
    for segment in &entry.prefix_segments {
        match segment {
            TreePrefixSegment::Pipe => prefix.push_str("│   "),
            TreePrefixSegment::Space => prefix.push_str("    "),
        }
    }

    // Continue the pipe from parent's connector position
    if entry.is_last_child {
        prefix.push_str("    │");
    } else {
        prefix.push_str("│   │");
    }

    prefix
}

/// Renders the entire UI.
pub fn render(frame: &mut Frame, app: &mut App) {
    let now = Utc::now();
    let area = frame.area();

    // Determine layout based on mode and error state
    let has_error = app.error_message.is_some();
    let is_search_mode = app.mode == AppMode::Search;
    // Show search bar only when text search is involved (search mode or confirmed query).
    // Status-only filter uses header highlighting, so no search bar needed.
    let has_text_filter = !app.confirmed_query.is_empty();
    let show_search_bar = is_search_mode || has_text_filter;

    let layouts: Vec<Constraint> = match (show_search_bar, has_error) {
        (true, true) => vec![
            Constraint::Length(3), // Header
            Constraint::Length(1), // Search bar (at top)
            Constraint::Min(1),    // Session list
            Constraint::Length(1), // Help bar
            Constraint::Length(1), // Error
        ],
        (true, false) => vec![
            Constraint::Length(3), // Header
            Constraint::Length(1), // Search bar (at top)
            Constraint::Min(1),    // Session list
            Constraint::Length(1), // Help bar
        ],
        (false, true) => vec![
            Constraint::Length(3), // Header
            Constraint::Min(1),    // Session list
            Constraint::Length(1), // Help bar
            Constraint::Length(1), // Error
        ],
        (false, false) => vec![
            Constraint::Length(3), // Header
            Constraint::Min(1),    // Session list
            Constraint::Length(1), // Help bar
        ],
    };

    let areas = Layout::vertical(layouts).split(area);

    render_header(frame, areas[0], app);

    match (show_search_bar, has_error) {
        (true, true) => {
            render_search_input(frame, areas[1], app);
            render_session_list(frame, areas[2], app, now);
            render_help(frame, areas[3], app);
            render_error(frame, areas[4], app.error_message.as_deref().unwrap_or(""));
        }
        (true, false) => {
            render_search_input(frame, areas[1], app);
            render_session_list(frame, areas[2], app, now);
            render_help(frame, areas[3], app);
        }
        (false, true) => {
            render_session_list(frame, areas[1], app, now);
            render_help(frame, areas[2], app);
            render_error(frame, areas[3], app.error_message.as_deref().unwrap_or(""));
        }
        (false, false) => {
            render_session_list(frame, areas[1], app, now);
            render_help(frame, areas[2], app);
        }
    }
}

/// Returns the style for a status indicator, highlighted when it matches the active filter.
fn get_status_style(
    base_color: Color,
    status: SessionStatus,
    active_filter: Option<SessionStatus>,
) -> Style {
    let style = Style::default().fg(base_color);
    if active_filter == Some(status) {
        style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    } else {
        style
    }
}

/// Renders the header with status counts.
/// When a status filter is active, the matching status is visually highlighted.
fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let (running, waiting, stopped) = count_statuses(&app.sessions);
    let status_filter = app.status_filter;

    let running_style = get_status_style(Color::Green, SessionStatus::Running, status_filter);
    let waiting_style = get_status_style(Color::Yellow, SessionStatus::WaitingInput, status_filter);
    let stopped_style = get_status_style(Color::DarkGray, SessionStatus::Stopped, status_filter);

    let status_line = Line::from(vec![
        Span::styled(
            "  Claude Code Sessions",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw("                       "),
        Span::styled(
            format!("{} {}", SessionStatus::Running.display_symbol(), running),
            running_style,
        ),
        Span::raw("  "),
        Span::styled(
            format!(
                "{} {}",
                SessionStatus::WaitingInput.display_symbol(),
                waiting
            ),
            waiting_style,
        ),
        Span::raw("  "),
        Span::styled(
            format!("{} {}", SessionStatus::Stopped.display_symbol(), stopped),
            stopped_style,
        ),
    ]);

    let header = Paragraph::new(status_line).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );

    frame.render_widget(header, area);
}

/// Renders the session list with tree view.
fn render_session_list(frame: &mut Frame, area: Rect, app: &mut App, now: DateTime<Utc>) {
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

    // Pre-compute repo names for filtered sessions. Populate cache
    // to avoid calling get_repo_root_in (filesystem I/O) on every
    // render frame. Must drop filtered_sessions first to release
    // the immutable borrow on app before mutating the cache.
    {
        let cwds: Vec<std::path::PathBuf> =
            filtered_sessions.iter().map(|s| s.cwd.clone()).collect();
        drop(filtered_sessions);
        app.ensure_repo_names_resolved(&cwds);
    }
    let filtered_sessions: Vec<&Session> = app.filtered_sessions();

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
            let repo_name = app.get_cached_repo_name(&entry.session.cwd).unwrap_or("");
            create_tree_session_item(
                entry,
                next_entry,
                cached_title,
                now,
                term_width,
                &query,
                repo_name,
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

/// Renders the help bar at the bottom.
fn render_help(frame: &mut Frame, area: Rect, app: &App) {
    let bold = Style::default().add_modifier(Modifier::BOLD);

    let help_text = match app.mode {
        AppMode::Search => Line::from(vec![
            Span::styled("  C-n/C-p", bold),
            Span::raw(": move  "),
            Span::styled("Enter", bold),
            Span::raw(": focus  "),
            Span::styled("Esc", bold),
            Span::raw(": cancel"),
        ]),
        AppMode::Normal if app.has_filter() => Line::from(vec![
            Span::styled("  j/k", bold),
            Span::raw(": move  "),
            Span::styled("Enter/f", bold),
            Span::raw(": focus  "),
            Span::styled("/", bold),
            Span::raw(": edit  "),
            Span::styled("r/w/s", bold),
            Span::raw(": filter  "),
            Span::styled("Esc", bold),
            Span::raw(": clear  "),
            Span::styled("q", bold),
            Span::raw(": quit"),
        ]),
        AppMode::Normal => Line::from(vec![
            Span::styled("  j/k", bold),
            Span::raw(": move  "),
            Span::styled("Enter/f", bold),
            Span::raw(": focus  "),
            Span::styled("1-9", bold),
            Span::raw(": quick  "),
            Span::styled("/", bold),
            Span::raw(": search  "),
            Span::styled("r/w/s", bold),
            Span::raw(": filter  "),
            Span::styled("q", bold),
            Span::raw(": quit"),
        ]),
    };

    let help = Paragraph::new(help_text).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, area);
}

/// Renders an error message at the bottom.
fn render_error(frame: &mut Frame, area: Rect, message: &str) {
    let error_text = Line::from(vec![
        Span::styled(
            "  Error: ",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled(message, Style::default().fg(Color::Red)),
    ]);

    let error = Paragraph::new(error_text);
    frame.render_widget(error, area);
}

/// Renders the search input bar.
fn render_search_input(frame: &mut Frame, area: Rect, app: &App) {
    let filtered_count = app.filtered_indices.len();
    let total_count = app.sessions.len();
    let count_str = format!("({}/{})", filtered_count, total_count);
    let term_width = area.width as usize;

    let is_search_mode = app.mode == AppMode::Search;

    // Use different query based on mode
    let query = if is_search_mode {
        &app.search_query
    } else {
        &app.confirmed_query
    };

    // Calculate available width for the search query
    let prefix = "  /";
    let cursor_str = if is_search_mode { "_" } else { "" };
    let count_width = count_str.len();
    let fixed_width = prefix.len() + cursor_str.len() + count_width + 2; // +2 for spacing
    let query_max_width = term_width.saturating_sub(fixed_width);

    // Truncate query if needed
    let display_query = truncate(query, query_max_width);

    // Calculate padding to right-align the count
    let content_width = prefix.len() + display_query.width() + cursor_str.len();
    let padding_width = term_width.saturating_sub(content_width + count_width + 2);
    let padding = " ".repeat(padding_width);

    let mut spans = vec![Span::styled(prefix, Style::default().fg(Color::Yellow))];

    spans.push(Span::styled(display_query, Style::default()));

    // Only show cursor in search mode
    if is_search_mode {
        spans.push(Span::styled(
            cursor_str,
            Style::default().add_modifier(Modifier::SLOW_BLINK),
        ));
    }

    spans.push(Span::raw(padding));
    spans.push(Span::styled(
        count_str,
        Style::default().fg(Color::DarkGray),
    ));

    let search_text = Line::from(spans);
    let search = Paragraph::new(search_text);
    frame.render_widget(search, area);
}

/// Creates a list item for a session within a tree view.
///
/// Each session renders as 2 lines:
/// - Line 1: [tree_prefix] status_symbol ▎ session_info  label  time_ago
/// - Line 2: [tree_prefix_continuation]  ▎ current_tool or last_message
///
/// Plus separator lines between tree entries (empty lines with connectors).
fn create_tree_session_item(
    entry: &TreeEntry,
    next_entry: Option<&TreeEntry>,
    cached_title: Option<&str>,
    now: DateTime<Utc>,
    term_width: usize,
    query: &str,
    repo_name: &str,
) -> ListItem<'static> {
    let session = entry.session;
    let status_symbol = session.status.display_symbol();
    let s_color = status_color(session.status);
    let session_info = get_session_info(session);
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
    let info_style = Style::default().add_modifier(Modifier::BOLD);

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
    line1_spans.push(Span::styled(time_ago, Style::default().fg(time_ago_fg)));
    let line1 = Line::from(line1_spans);

    // Line 2: [tree_prefix_continuation]  ▎ current_tool or last_message
    let line2_content = session
        .current_tool
        .as_deref()
        .or(session.last_message.as_deref())
        .unwrap_or("");
    let truncated_content = truncate(line2_content, content_width);
    let content_style = Style::default().add_modifier(Modifier::DIM);
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

/// Returns a deterministic color for a repository name.
/// Uses FNV-1a hash for good distribution, then maps to a hue on the
/// HSL color wheel with fixed saturation and lightness for terminal
/// readability.
fn repo_label_color(repo_name: &str) -> Color {
    // FNV-1a hash for better distribution than simple multiply-add
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x00000100000001B3;
    // FNV-1a: XOR first, then multiply
    let hash = repo_name.bytes().fold(FNV_OFFSET, |acc, b| {
        (acc ^ (b as u64)).wrapping_mul(FNV_PRIME)
    });

    let slot = (hash % REPO_LABEL_HUE_SLOTS) as f64;
    let hue = (slot / REPO_LABEL_HUE_SLOTS as f64) * 360.0;
    let (r, g, b) = hsl_to_rgb(hue, 0.65, 0.45);
    Color::Rgb(r, g, b)
}

/// Converts HSL color to RGB (each component 0-255).
fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h_prime = h / 60.0;
    let x = c * (1.0 - (h_prime % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match h_prime as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    (
        ((r1 + m) * 255.0) as u8,
        ((g1 + m) * 255.0) as u8,
        ((b1 + m) * 255.0) as u8,
    )
}

/// Returns the color for a session status.
fn status_color(status: SessionStatus) -> Color {
    match status {
        SessionStatus::Running => Color::Green,
        SessionStatus::WaitingInput => Color::Yellow,
        SessionStatus::Stopped => Color::DarkGray,
    }
}

/// Returns a color for the "time ago" label based on elapsed time since last update.
///
/// Uses a green-to-gray gradient: bright green for recent activity, fading
/// through darker greens, then to gray for stale sessions.
fn time_ago_color(updated_at: DateTime<Utc>, now: DateTime<Utc>) -> Color {
    let seconds = now.signed_duration_since(updated_at).num_seconds().max(0);
    let minutes = seconds / 60;

    match minutes {
        // < 1 minute: bright green
        0 => TIME_AGO_BRIGHT_GREEN,
        // 1-5 minutes: green
        1..=5 => TIME_AGO_GREEN,
        // 6-30 minutes: slightly darker green
        6..=30 => TIME_AGO_DARK_GREEN,
        // 31-60 minutes: dark green
        31..=60 => TIME_AGO_DARKER_GREEN,
        // 1-6 hours: light gray
        61..=360 => TIME_AGO_LIGHT_GRAY,
        // 6+ hours: darker gray (floor)
        _ => TIME_AGO_DARK_GRAY,
    }
}

/// Counts sessions by status.
fn count_statuses(sessions: &[Session]) -> (usize, usize, usize) {
    let mut running = 0;
    let mut waiting = 0;
    let mut stopped = 0;

    for session in sessions {
        match session.status {
            SessionStatus::Running => running += 1,
            SessionStatus::WaitingInput => waiting += 1,
            SessionStatus::Stopped => stopped += 1,
        }
    }

    (running, waiting, stopped)
}

/// Gets the title display name for a session without external file I/O.
/// Used as fallback when title is not in cache.
/// All outputs are sanitized to strip ANSI escape sequences.
fn get_title_display_name_fallback(session: &Session) -> String {
    use crate::commands::cc::claude_sessions;

    if let Some(ref label) = session.label {
        return claude_sessions::normalize_title(label);
    }

    if let Some(ref tmux_info) = session.tmux_info {
        return claude_sessions::normalize_title(&format!(
            "{}:{}",
            tmux_info.session_name, tmux_info.window_name
        ));
    }

    // Extract last component of cwd path
    let raw_title = session
        .cwd
        .file_name()
        .and_then(|n| n.to_str())
        .map(String::from)
        .unwrap_or_else(|| session.cwd.display().to_string());
    claude_sessions::normalize_title(&raw_title)
}

/// Gets the session info (tmux session:window or cwd path).
/// All outputs are sanitized to strip ANSI escape sequences.
fn get_session_info(session: &Session) -> String {
    use crate::commands::cc::claude_sessions;

    let raw = if let Some(ref tmux_info) = session.tmux_info {
        format!("{}:{}", tmux_info.session_name, tmux_info.window_name)
    } else {
        session.cwd.display().to_string()
    };
    claude_sessions::normalize_title(&raw)
}

/// Formats a datetime as a relative time string.
fn format_relative_time(dt: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let duration = now.signed_duration_since(dt);

    let seconds = duration.num_seconds();
    if seconds < 0 {
        return "just now".to_string();
    }

    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;

    if seconds < 60 {
        "just now".to_string()
    } else if minutes < 60 {
        format!("{}m ago", minutes)
    } else if hours < 24 {
        format!("{}h ago", hours)
    } else {
        format!("{}d ago", days)
    }
}

/// Splits text into spans, highlighting portions that match any of the search words.
///
/// Uses case-insensitive matching consistent with the existing search logic.
/// When multiple words produce overlapping match ranges, they are merged.
/// Highlighted spans receive `fg(Color::Yellow)` and `BOLD` on top of `base_style`.
fn highlight_matches<'a>(text: &str, query: &str, base_style: Style) -> Vec<Span<'a>> {
    let words: Vec<&str> = query.split_whitespace().collect();
    if words.is_empty() || text.is_empty() {
        return vec![Span::styled(text.to_string(), base_style)];
    }

    // Build a mapping from lowercased byte offsets to original byte offsets.
    // to_lowercase() can change byte length (e.g. 'İ' -> "i\u{307}"),
    // so we must map match positions in the lowercased string back to the original.
    let mut text_lower = String::new();
    let mut lower_to_orig: Vec<usize> = Vec::new();
    for (orig_offset, ch) in text.char_indices() {
        for lower_ch in ch.to_lowercase() {
            let lower_start = text_lower.len();
            text_lower.push(lower_ch);
            // Map each byte of the lowercased char to the original char's byte offset
            for _ in lower_start..text_lower.len() {
                lower_to_orig.push(orig_offset);
            }
        }
    }
    // Sentinel: map end-of-lowered-string to end-of-original-string
    lower_to_orig.push(text.len());

    // Collect all match ranges (byte offsets in the original string)
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for word in &words {
        let word_lower = word.to_lowercase();
        let mut start = 0;
        while let Some(pos) = text_lower[start..].find(&word_lower) {
            let lower_start = start + pos;
            let lower_end = lower_start + word_lower.len();
            let orig_start = lower_to_orig[lower_start];
            let mut orig_end = lower_to_orig[lower_end];
            // When lowercasing expands bytes (e.g. İ -> i\u{307}), a match
            // within the expansion maps start and end to the same original
            // offset. Extend to cover the full original character.
            if orig_end <= orig_start {
                orig_end = text[orig_start..]
                    .chars()
                    .next()
                    .map_or(text.len(), |c| orig_start + c.len_utf8());
            }
            ranges.push((orig_start, orig_end));
            // Advance by one character (not one byte) to stay on a char boundary
            start = lower_start
                + text_lower[lower_start..]
                    .chars()
                    .next()
                    .map_or(1, |c| c.len_utf8());
        }
    }

    if ranges.is_empty() {
        return vec![Span::styled(text.to_string(), base_style)];
    }

    // Sort by start position, then by end position descending to prefer longer matches
    ranges.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));

    // Merge overlapping ranges
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for range in ranges {
        if let Some(last) = merged.last_mut()
            && range.0 <= last.1
        {
            last.1 = last.1.max(range.1);
            continue;
        }
        merged.push(range);
    }

    // Build spans from merged ranges
    let highlight_style = base_style.fg(Color::Yellow).add_modifier(Modifier::BOLD);
    let mut spans = Vec::new();
    let mut cursor = 0;

    for (start, end) in merged {
        if cursor < start {
            spans.push(Span::styled(text[cursor..start].to_string(), base_style));
        }
        spans.push(Span::styled(text[start..end].to_string(), highlight_style));
        cursor = end;
    }

    if cursor < text.len() {
        spans.push(Span::styled(text[cursor..].to_string(), base_style));
    }

    spans
}

/// Truncates a string to fit within the specified display width.
/// Uses unicode display width for proper handling of CJK characters.
fn truncate(s: &str, max_width: usize) -> String {
    let display_width = s.width();
    if display_width <= max_width {
        s.to_string()
    } else if max_width < 3 {
        truncate_to_width(s, max_width)
    } else {
        let truncated = truncate_to_width(s, max_width - 3);
        format!("{}...", truncated)
    }
}

/// Truncates a string to fit within the specified display width.
fn truncate_to_width(s: &str, max_width: usize) -> String {
    let mut result = String::new();
    let mut current_width = 0;

    for c in s.chars() {
        let char_width = c.width().unwrap_or(0);
        if current_width + char_width > max_width {
            break;
        }
        result.push(c);
        current_width += char_width;
    }

    result
}

/// Renders the entire UI to a TestBackend for testing.
/// Returns the rendered output as a string.
#[cfg(test)]
fn render_to_string(
    sessions: &[Session],
    selected_index: Option<usize>,
    now: DateTime<Utc>,
    width: u16,
    height: u16,
) -> String {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();

    // Create a minimal App state for rendering
    let mut list_state = ListState::default();
    list_state.select(selected_index);

    let mut app = App::with_sessions(sessions.to_vec());
    app.list_state = list_state;

    terminal
        .draw(|frame| {
            let area = frame.area();
            let areas = Layout::vertical([
                Constraint::Length(3),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(area);

            render_header(frame, areas[0], &app);
            render_session_list_internal(frame, areas[1], sessions, &mut app.list_state, now);
            render_help(frame, areas[2], &app);
        })
        .unwrap();

    // Convert buffer to string
    let backend = terminal.backend();
    let buffer = backend.buffer();
    let mut output = String::new();

    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, y)];
            output.push_str(cell.symbol());
        }
        // Trim trailing whitespace and add newline
        let trimmed = output.trim_end_matches(' ');
        output.truncate(trimmed.len());
        output.push('\n');
    }

    // Remove trailing newline
    if output.ends_with('\n') {
        output.pop();
    }

    output
}

/// Internal render function for session list used by test render.
/// Uses tree view rendering.
#[cfg(test)]
fn render_session_list_internal(
    frame: &mut Frame,
    area: Rect,
    sessions: &[Session],
    list_state: &mut ListState,
    now: DateTime<Utc>,
) {
    if sessions.is_empty() {
        let empty_message = Paragraph::new("  No active Claude Code sessions.")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(empty_message, area);
        return;
    }

    let term_width = area.width as usize;
    let session_refs: Vec<&Session> = sessions.iter().collect();
    let tree_entries = build_session_tree(&session_refs);

    let items: Vec<ListItem> = tree_entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let next_entry = tree_entries.get(i + 1);
            let title = get_title_display_name_fallback(entry.session);
            let repo_name = entry
                .session
                .cwd
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            create_tree_session_item(
                entry,
                next_entry,
                Some(&title),
                now,
                term_width,
                "",
                repo_name,
            )
        })
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol(">");

    frame.render_stateful_widget(list, area, list_state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::types::TmuxInfo;
    use chrono::Duration;
    use rstest::rstest;
    use std::path::PathBuf;

    fn create_test_session(id: &str) -> Session {
        Session {
            session_id: id.to_string(),
            cwd: PathBuf::from("/home/user/project"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
        }
    }

    #[test]
    fn test_count_statuses() {
        let sessions = vec![
            {
                let mut s = create_test_session("1");
                s.status = SessionStatus::Running;
                s
            },
            {
                let mut s = create_test_session("2");
                s.status = SessionStatus::Running;
                s
            },
            {
                let mut s = create_test_session("3");
                s.status = SessionStatus::WaitingInput;
                s
            },
            {
                let mut s = create_test_session("4");
                s.status = SessionStatus::Stopped;
                s
            },
        ];

        let (running, waiting, stopped) = count_statuses(&sessions);
        assert_eq!(running, 2);
        assert_eq!(waiting, 1);
        assert_eq!(stopped, 1);
    }

    #[test]
    fn test_get_title_display_name_fallback_to_tmux() {
        // When cache is not available, falls back to tmux info
        let mut session = create_test_session("test");
        session.tmux_info = Some(TmuxInfo {
            session_name: "dev".to_string(),
            window_name: "editor".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        });
        assert_eq!(get_title_display_name_fallback(&session), "dev:editor");
    }

    #[test]
    fn test_get_title_display_name_fallback_to_cwd() {
        // When cache is not available and no tmux, falls back to cwd
        let session = create_test_session("test");
        assert_eq!(get_title_display_name_fallback(&session), "project");
    }

    #[test]
    fn test_get_session_info_with_tmux() {
        let mut session = create_test_session("test");
        session.tmux_info = Some(TmuxInfo {
            session_name: "dev".to_string(),
            window_name: "editor".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        });
        assert_eq!(get_session_info(&session), "dev:editor");
    }

    #[test]
    fn test_get_session_info_without_tmux() {
        let session = create_test_session("test");
        assert_eq!(get_session_info(&session), "/home/user/project");
    }

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

    #[rstest]
    #[case::short("hello", 10, "hello")]
    #[case::exact("hello", 5, "hello")]
    #[case::truncate("hello world", 8, "hello...")]
    #[case::cjk_short("日本語", 10, "日本語")]
    #[case::cjk_exact("日本語", 6, "日本語")]
    #[case::cjk_truncate("日本語テスト", 8, "日本...")]
    fn test_truncate(#[case] input: &str, #[case] max_width: usize, #[case] expected: &str) {
        assert_eq!(truncate(input, max_width), expected);
    }

    #[test]
    fn test_status_color() {
        assert_eq!(status_color(SessionStatus::Running), Color::Green);
        assert_eq!(status_color(SessionStatus::WaitingInput), Color::Yellow);
        assert_eq!(status_color(SessionStatus::Stopped), Color::DarkGray);
    }

    #[rstest]
    #[case::just_now(0, Color::Indexed(82))]
    #[case::thirty_seconds(30, Color::Indexed(82))]
    #[case::almost_one_minute(59, Color::Indexed(82))]
    #[case::one_minute(60, Color::Indexed(78))]
    #[case::three_minutes(180, Color::Indexed(78))]
    #[case::five_minutes(300, Color::Indexed(78))]
    #[case::six_minutes(360, Color::Indexed(72))]
    #[case::fifteen_minutes(900, Color::Indexed(72))]
    #[case::thirty_minutes(1800, Color::Indexed(72))]
    #[case::thirty_one_minutes(1860, Color::Indexed(65))]
    #[case::forty_five_minutes(2700, Color::Indexed(65))]
    #[case::one_hour(3600, Color::Indexed(65))]
    #[case::two_hours(7200, Color::Indexed(245))]
    #[case::five_hours(18000, Color::Indexed(245))]
    #[case::six_hours(21600, Color::Indexed(245))]
    #[case::seven_hours(25200, Color::Indexed(241))]
    #[case::one_day(86400, Color::Indexed(241))]
    fn test_time_ago_color(#[case] seconds_ago: i64, #[case] expected: Color) {
        let now = Utc::now();
        let updated_at = now - Duration::seconds(seconds_ago);
        assert_eq!(time_ago_color(updated_at, now), expected);
    }

    // =========================================================================
    // Full-screen integration tests using TestBackend (tree view)
    // =========================================================================

    /// Helper to check that rendered output contains expected substrings.
    /// Each expected string must appear as a substring within some line of the output.
    fn assert_output_contains(output: &str, expected_substrings: &[&str]) {
        for expected in expected_substrings {
            assert!(
                output.lines().any(|line| line.contains(expected)),
                "Expected substring not found in any line:\n  expected: {:?}\n  output:\n{}",
                expected,
                output
            );
        }
    }

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
        let output = render_to_string(&sessions, Some(0), now, 80, 12);

        // Flat sessions (no parent-child): each is a root
        // Line format: "{status} ▎ {session_info}  {time_ago}"
        // Label is omitted when equal to session_info
        assert_output_contains(
            &output,
            &[">● ▎ webapp:dev  just now", "◐ ▎ api:test  5m ago"],
        );
    }

    #[test]
    fn test_render_full_screen_empty_sessions() {
        let now = Utc::now();
        let sessions: Vec<Session> = vec![];
        let output = render_to_string(&sessions, None, now, 80, 8);

        assert_output_contains(&output, &["No active Claude Code sessions."]);
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
        let output = render_to_string(&sessions, Some(0), now, 80, 8);

        // Line 2 should contain the last_message after the bar
        assert_output_contains(
            &output,
            &[
                ">● ▎ webapp:dev  just now",
                "▎ I've updated the code as requested.",
            ],
        );
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
        let output = render_to_string(&sessions, Some(0), now, 80, 8);

        // Line 1 should have status + bar + session_info
        assert_output_contains(&output, &[">● ▎ webapp:dev  just now"]);
        // Line 2 should have only the bar (no content)
        let output_lines: Vec<&str> = output.lines().collect();
        // Find a line that is just whitespace + ▎ (the continuation line)
        assert!(
            output_lines.iter().any(|line| {
                let trimmed = line.trim();
                trimmed == "▎"
            }),
            "Expected a line with only the bar character"
        );
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
        let output = render_to_string(&sessions, Some(0), now, 80, 8);

        assert_output_contains(&output, &["○ ▎ /home/user/docs  docs"]);
    }

    // =========================================================================
    // Tree view structure tests
    // =========================================================================

    #[test]
    fn test_build_session_tree_flat_sessions() {
        let s1 = create_test_session("a");
        let s2 = create_test_session("b");
        let sessions: Vec<&Session> = vec![&s1, &s2];

        let tree = build_session_tree(&sessions);

        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].depth, 0);
        assert_eq!(tree[1].depth, 0);
        assert!(!tree[0].has_children);
        assert!(!tree[1].has_children);
    }

    #[test]
    fn test_build_session_tree_parent_child() {
        let parent = create_test_session("parent");
        let mut child = create_test_session("child");
        child.ancestor_session_ids = vec!["parent".to_string()];

        let sessions: Vec<&Session> = vec![&parent, &child];
        let tree = build_session_tree(&sessions);

        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].session.session_id, "parent");
        assert_eq!(tree[0].depth, 0);
        assert!(tree[0].has_children);
        assert_eq!(tree[1].session.session_id, "child");
        assert_eq!(tree[1].depth, 1);
        assert!(tree[1].is_last_child);
    }

    #[test]
    fn test_build_session_tree_skips_deleted_ancestor() {
        // ancestor_session_ids = [root, deleted_middle]
        // Only root is displayed, so child should attach to root
        let root = create_test_session("root");
        let mut child = create_test_session("child");
        child.ancestor_session_ids = vec!["root".to_string(), "deleted_middle".to_string()];

        let sessions: Vec<&Session> = vec![&root, &child];
        let tree = build_session_tree(&sessions);

        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].session.session_id, "root");
        assert!(tree[0].has_children);
        assert_eq!(tree[1].session.session_id, "child");
        assert_eq!(tree[1].depth, 1);
    }

    #[test]
    fn test_build_session_tree_multiple_children() {
        let parent = create_test_session("parent");
        let mut child1 = create_test_session("child1");
        child1.ancestor_session_ids = vec!["parent".to_string()];
        let mut child2 = create_test_session("child2");
        child2.ancestor_session_ids = vec!["parent".to_string()];

        let sessions: Vec<&Session> = vec![&parent, &child1, &child2];
        let tree = build_session_tree(&sessions);

        assert_eq!(tree.len(), 3);
        assert_eq!(tree[0].depth, 0);
        assert!(tree[0].has_children);
        // child1: not last child
        assert_eq!(tree[1].depth, 1);
        assert!(!tree[1].is_last_child);
        // child2: last child
        assert_eq!(tree[2].depth, 1);
        assert!(tree[2].is_last_child);
    }

    #[test]
    fn test_build_session_tree_nested() {
        let root = create_test_session("root");
        let mut mid = create_test_session("mid");
        mid.ancestor_session_ids = vec!["root".to_string()];
        let mut leaf = create_test_session("leaf");
        leaf.ancestor_session_ids = vec!["root".to_string(), "mid".to_string()];

        let sessions: Vec<&Session> = vec![&root, &mid, &leaf];
        let tree = build_session_tree(&sessions);

        assert_eq!(tree.len(), 3);
        assert_eq!(tree[0].depth, 0); // root
        assert_eq!(tree[1].depth, 1); // mid
        assert_eq!(tree[2].depth, 2); // leaf
    }

    // =========================================================================
    // Tree prefix tests
    // =========================================================================

    #[test]
    fn test_tree_prefix_root_node() {
        let session = create_test_session("root");
        let entry = TreeEntry {
            session: &session,
            depth: 0,
            is_last_child: true,
            prefix_segments: vec![],
            has_children: false,
        };

        assert_eq!(build_line1_tree_prefix(&entry), "");
        assert_eq!(build_line2_tree_prefix(&entry), "");
    }

    #[test]
    fn test_tree_prefix_first_child() {
        let session = create_test_session("child");
        let entry = TreeEntry {
            session: &session,
            depth: 1,
            is_last_child: false,
            prefix_segments: vec![],
            has_children: false,
        };

        assert_eq!(build_line1_tree_prefix(&entry), "├── ");
        assert_eq!(build_line2_tree_prefix(&entry), "│   ");
    }

    #[test]
    fn test_tree_prefix_last_child() {
        let session = create_test_session("child");
        let entry = TreeEntry {
            session: &session,
            depth: 1,
            is_last_child: true,
            prefix_segments: vec![],
            has_children: false,
        };

        assert_eq!(build_line1_tree_prefix(&entry), "└── ");
        assert_eq!(build_line2_tree_prefix(&entry), "    ");
    }

    #[test]
    fn test_tree_prefix_nested_with_pipe() {
        let session = create_test_session("deep");
        let entry = TreeEntry {
            session: &session,
            depth: 2,
            is_last_child: true,
            prefix_segments: vec![TreePrefixSegment::Pipe],
            has_children: false,
        };

        assert_eq!(build_line1_tree_prefix(&entry), "│   └── ");
        assert_eq!(build_line2_tree_prefix(&entry), "│       ");
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
        let output = render_to_string(&sessions, Some(0), now, 80, 12);

        // Parent should show without tree prefix (root)
        // Child should show with tree prefix (└── )
        assert_output_contains(
            &output,
            &[
                ">● ▎ app:main  just now",
                "▎ Bash(cargo build)",
                "└── ● ▎ app:test  2m ago",
                "▎ Bash(cargo test)",
            ],
        );
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
        let output = render_to_string(&sessions, Some(0), now, 80, 15);

        // First child should have ├── , last child should have └──
        assert_output_contains(
            &output,
            &[
                ">● ▎ app:main  just now",
                "├── ● ▎ app:test  1m ago",
                "└── ◐ ▎ app:review  3m ago",
            ],
        );
    }

    // =========================================================================
    // highlight_matches tests
    // =========================================================================

    /// Each expected element is (content, is_highlighted).
    #[rstest]
    #[case::empty_query("webapp:dev", "", &[("webapp:dev", false)])]
    #[case::no_match("webapp:dev", "xyz", &[("webapp:dev", false)])]
    #[case::single_word("webapp:dev", "web", &[("web", true), ("app:dev", false)])]
    #[case::case_insensitive("WebApp", "web", &[("Web", true), ("App", false)])]
    #[case::multiple_words("webapp:dev", "web dev", &[("web", true), ("app:", false), ("dev", true)])]
    #[case::overlapping_ranges("abcd", "ab bc", &[("abc", true), ("d", false)])]
    #[case::multiple_occurrences("abcabc", "ab", &[("ab", true), ("c", false), ("ab", true), ("c", false)])]
    #[case::empty_text("", "web", &[("", false)])]
    #[case::unicode_byte_length_increase("İstanbul City", "city", &[("İstanbul ", false), ("City", true)])]
    #[case::unicode_byte_length_decrease("\u{212A}elvin", "kelvin", &[("\u{212A}elvin", true)])]
    #[case::multibyte_overlapping_match("ああいああ", "ああ", &[("ああ", true), ("い", false), ("ああ", true)])]
    fn test_highlight_matches(
        #[case] text: &str,
        #[case] query: &str,
        #[case] expected: &[(&str, bool)],
    ) {
        let base = Style::default().add_modifier(Modifier::BOLD);
        let highlight = base.fg(Color::Yellow).add_modifier(Modifier::BOLD);

        let spans = highlight_matches(text, query, base);

        assert_eq!(spans.len(), expected.len(), "span count mismatch");
        for (span, &(content, is_highlighted)) in spans.iter().zip(expected) {
            assert_eq!(span.content, content);
            assert_eq!(span.style, if is_highlighted { highlight } else { base });
        }
    }

    // =========================================================================
    // Repo label tests
    // =========================================================================

    #[test]
    fn test_repo_label_color_is_deterministic() {
        let color1 = repo_label_color("armyknife");
        let color2 = repo_label_color("armyknife");
        assert_eq!(color1, color2);
    }

    #[test]
    fn test_repo_label_color_differs_for_different_names() {
        let color1 = repo_label_color("armyknife");
        let color2 = repo_label_color("specs");
        assert_ne!(color1, color2);
    }

    #[test]
    fn test_repo_label_color_returns_rgb() {
        let names = ["armyknife", "webapp", "api", "docs", "infra", "tools"];
        for name in names {
            let color = repo_label_color(name);
            assert!(
                matches!(color, Color::Rgb(_, _, _)),
                "{name} got {color:?}, expected Rgb"
            );
        }
    }
}
