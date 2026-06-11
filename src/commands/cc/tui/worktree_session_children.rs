//! Shared "sessions living under a worktree" model used by both the
//! worktree view and the clean view.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::ListItem;

use crate::commands::cc::types::{Session, SessionStatus};

#[cfg(test)]
use super::worktree_view::canonicalize_or_self;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionChild {
    pub session_id: String,
    /// `None` when the session has no tmux pane (e.g. resurrect-only state).
    pub pane_id: Option<String>,
    pub status: SessionStatus,
    pub updated_at: DateTime<Utc>,
    /// `label` if present, otherwise the cwd basename.
    pub label: String,
    /// True when this is the last session under its parent worktree —
    /// drives `└─` vs `├─` tree connectors.
    pub is_last: bool,
}

/// Collects sessions whose cwd lives under `worktree_path`, sorted
/// newest-first by `updated_at`. Takes pre-canonicalized `(cwd, session)`
/// pairs so the caller can amortize canonicalize across many worktree
/// rows; macOS `/tmp` vs `/private/tmp` resolves correctly as long as
/// both `worktree_path` and each cwd in `canonical_sessions` were run
/// through [`canonicalize_or_self`](super::worktree_view::canonicalize_or_self).
pub fn sessions_under_worktree_from_canonical(
    worktree_path: &Path,
    canonical_sessions: &[(PathBuf, &Session)],
) -> Vec<SessionChild> {
    let mut matched: Vec<&Session> = canonical_sessions
        .iter()
        .filter(|(c, _)| c.starts_with(worktree_path))
        .map(|(_, s)| *s)
        .collect();
    matched.sort_by_key(|s| std::cmp::Reverse(s.updated_at));

    let last_idx = matched.len().saturating_sub(1);
    matched
        .into_iter()
        .enumerate()
        .map(|(i, s)| SessionChild {
            session_id: s.session_id.clone(),
            pane_id: s.tmux_info.as_ref().map(|t| t.pane_id.clone()),
            status: s.status,
            updated_at: s.updated_at,
            label: short_label(s),
            is_last: i == last_idx,
        })
        .collect()
}

fn short_label(session: &Session) -> String {
    if let Some(label) = session.label.as_deref()
        && !label.is_empty()
    {
        return label.to_string();
    }
    session
        .cwd
        .file_name()
        .and_then(|n| n.to_str())
        .map(String::from)
        .unwrap_or_else(|| session.cwd.display().to_string())
}

fn status_color(status: SessionStatus) -> Color {
    match status {
        SessionStatus::Running => Color::Green,
        SessionStatus::WaitingInput => Color::Yellow,
        SessionStatus::Paused => Color::Indexed(245),
        SessionStatus::Stopped | SessionStatus::Ended => Color::DarkGray,
    }
}

/// Renders one session-child row: `    <connector> <status> <label>  <time-ago>`.
/// The leading spaces align the connector under the `▎` bar on the parent
/// worktree row's second line.
pub fn create_session_child_list_item(
    child: &SessionChild,
    now: DateTime<Utc>,
) -> ListItem<'static> {
    let connector = if child.is_last { "└─" } else { "├─" };
    let symbol = child.status.display_symbol();
    let s_style = Style::default().fg(status_color(child.status));
    let dim = Style::default().fg(Color::DarkGray);
    let time_ago = format_relative_time(child.updated_at, now);

    let line = Line::from(vec![
        Span::raw("    "),
        Span::styled(connector.to_string(), dim),
        Span::raw(" "),
        Span::styled(symbol.to_string(), s_style),
        Span::raw(" "),
        Span::styled(
            child.label.clone(),
            Style::default().add_modifier(Modifier::DIM),
        ),
        Span::raw("  "),
        Span::styled(time_ago, dim),
    ]);

    ListItem::new(vec![line])
}

fn format_relative_time(dt: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let duration = now.signed_duration_since(dt);
    let seconds = duration.num_seconds();
    if seconds < 60 {
        return "just now".to_string();
    }
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    if minutes < 60 {
        format!("{minutes}m ago")
    } else if hours < 24 {
        format!("{hours}h ago")
    } else {
        format!("{days}d ago")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::types::TmuxInfo;
    use rstest::{fixture, rstest};
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn session(id: &str, cwd: &Path, updated_at: DateTime<Utc>, pane: Option<&str>) -> Session {
        Session {
            session_id: id.to_string(),
            cwd: cwd.to_path_buf(),
            transcript_path: None,
            tty: None,
            tmux_info: pane.map(|p| TmuxInfo {
                session_name: "s".to_string(),
                window_name: "w".to_string(),
                window_index: 0,
                pane_id: p.to_string(),
            }),
            status: SessionStatus::Running,
            created_at: updated_at,
            updated_at,
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: BTreeSet::new(),
        }
    }

    #[fixture]
    fn tmpdir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn canonical_pairs(sessions: &[Session]) -> Vec<(PathBuf, &Session)> {
        sessions
            .iter()
            .map(|s| (canonicalize_or_self(&s.cwd), s))
            .collect()
    }

    #[rstest]
    fn sessions_under_worktree_filters_and_sorts_newest_first(tmpdir: tempfile::TempDir) {
        let wt = tmpdir.path().join("wt");
        let other = tmpdir.path().join("other");
        std::fs::create_dir_all(&wt).expect("mkdir wt");
        std::fs::create_dir_all(&other).expect("mkdir other");

        let t0 = Utc::now();
        let sessions = vec![
            session("old", &wt, t0 - chrono::Duration::seconds(120), Some("%1")),
            session("new", &wt, t0, Some("%2")),
            session("outside", &other, t0, Some("%3")),
        ];

        let result = sessions_under_worktree_from_canonical(
            &canonicalize_or_self(&wt),
            &canonical_pairs(&sessions),
        );

        assert_eq!(
            result,
            vec![
                SessionChild {
                    session_id: "new".to_string(),
                    pane_id: Some("%2".to_string()),
                    status: SessionStatus::Running,
                    updated_at: t0,
                    label: "wt".to_string(),
                    is_last: false,
                },
                SessionChild {
                    session_id: "old".to_string(),
                    pane_id: Some("%1".to_string()),
                    status: SessionStatus::Running,
                    updated_at: t0 - chrono::Duration::seconds(120),
                    label: "wt".to_string(),
                    is_last: true,
                },
            ],
        );
    }

    #[rstest]
    fn sessions_under_worktree_returns_empty_when_no_match(tmpdir: tempfile::TempDir) {
        let wt = tmpdir.path().join("wt");
        std::fs::create_dir_all(&wt).expect("mkdir");
        let t0 = Utc::now();
        let sessions = vec![session("s", &PathBuf::from("/elsewhere"), t0, None)];
        assert_eq!(
            sessions_under_worktree_from_canonical(
                &canonicalize_or_self(&wt),
                &canonical_pairs(&sessions),
            ),
            Vec::<SessionChild>::new(),
        );
    }

    #[rstest]
    fn short_label_prefers_session_label() {
        let mut s = session("s", Path::new("/tmp/wt"), Utc::now(), None);
        s.label = Some("my-label".to_string());
        assert_eq!(short_label(&s), "my-label");
    }

    #[rstest]
    fn short_label_falls_back_to_cwd_basename() {
        let s = session("s", Path::new("/tmp/some-worktree"), Utc::now(), None);
        assert_eq!(short_label(&s), "some-worktree");
    }
}
