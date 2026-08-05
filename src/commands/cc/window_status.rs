use std::collections::HashSet;
use std::io::{self, Write};
use std::path::Path;

use anyhow::Result;
use clap::Args;

use super::store;
use super::types::{
    Session, SessionStatus, TMUX_SESSION_OPTION, TMUX_WINDOW_STATUS_OPTION,
    TMUX_WINDOW_TITLE_OPTION,
};
use crate::infra::tmux;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct WindowStatusArgs {
    /// Tmux window ID to inspect (e.g. `@3`)
    pub window_id: String,
}

/// Runs the window-status command.
///
/// Prints the status symbols of every Claude Code session running in the
/// panes of the given tmux window. The event-driven path writes the same
/// string to the window's `@armyknife-cc-window-status` option (see
/// `sync_window_option`); this command exists for manual inspection and for
/// a polling-based `window-status-format` that calls `#(a cc window-status)`.
pub fn run(args: &WindowStatusArgs) -> Result<()> {
    let sessions = load_window_sessions(&args.window_id, &store::sessions_dir()?)?;

    let mut stdout = io::stdout().lock();
    write!(stdout, "{}", render_window_status(&sessions))?;

    Ok(())
}

/// Recomputes `window_id`'s aggregated Claude Code status symbols and title,
/// and writes each to its own window option (`@armyknife-cc-window-status`,
/// `@armyknife-cc-window-title`).
///
/// Each option's write is skipped independently when its rendered value
/// matches what tmux already holds, so an event that only changes one of the
/// two (e.g. a status transition with no rename) costs a single option write.
/// The status-bar refresh still runs at most once, if either option changed.
/// This is what turns the per-redraw polling of `#(a cc window-status)` into
/// an event-driven update fired only by `a cc hook`.
/// Best-effort push of the tmux window title option after `session`'s
/// `label` changed. Silently does nothing without a live tmux pane for
/// `session`, an unresolvable window, or an unavailable sessions
/// directory -- this must never fail the caller (a manual rename confirm,
/// or a detached title-generation write).
pub fn sync_window_title_for_session(session: &Session) {
    let Some(pane_id) = session.tmux_info.as_ref().map(|info| info.pane_id.as_str()) else {
        return;
    };
    let Some(window_id) = tmux::get_window_id_for_pane(pane_id) else {
        return;
    };
    let Ok(sessions_dir) = store::sessions_dir() else {
        return;
    };
    let _ = sync_window_option(&window_id, &sessions_dir);
}

pub fn sync_window_option(window_id: &str, sessions_dir: &Path) -> Result<()> {
    let sessions = load_window_sessions(window_id, sessions_dir)?;

    let rendered_status = render_window_status(&sessions);
    let current_status = tmux::get_window_option(window_id, TMUX_WINDOW_STATUS_OPTION);
    let status_changed = tmux_option_changed(current_status.as_deref(), &rendered_status);

    let rendered_title = render_window_title(&sessions);
    let current_title = tmux::get_window_option(window_id, TMUX_WINDOW_TITLE_OPTION);
    let title_changed = tmux_option_changed(current_title.as_deref(), &rendered_title);

    if !status_changed && !title_changed {
        return Ok(());
    }

    if status_changed {
        tmux::set_window_option(window_id, TMUX_WINDOW_STATUS_OPTION, &rendered_status)?;
    }
    if title_changed {
        tmux::set_window_option(window_id, TMUX_WINDOW_TITLE_OPTION, &rendered_title)?;
    }
    tmux::refresh_status()?;

    Ok(())
}

/// Loads every distinct Claude Code session running in the panes of `window_id`.
///
/// Sessions are resolved via each pane's session-id option, so the cost stays
/// O(panes in window) rather than O(all sessions on disk). Two panes can carry
/// the same session id (e.g. a split pane keeps the option), so duplicates are
/// dropped to avoid rendering a session's symbol twice.
fn load_window_sessions(window_id: &str, sessions_dir: &Path) -> Result<Vec<Session>> {
    let session_ids = tmux::list_window_pane_options(window_id, TMUX_SESSION_OPTION);

    let mut seen = HashSet::new();
    let mut sessions = Vec::with_capacity(session_ids.len());
    for session_id in &session_ids {
        if !seen.insert(session_id.as_str()) {
            continue;
        }
        if let Some(session) = store::load_session_from(sessions_dir, session_id)? {
            sessions.push(session);
        }
    }

    Ok(sessions)
}

/// Renders the aggregated status symbols for a window's sessions.
///
/// Each session contributes one symbol, in the given order, with no
/// separator. When at least one symbol is emitted, a trailing space is added
/// so the result reads cleanly when prepended to a window name. Returns an
/// empty string when no session contributes a symbol.
fn render_window_status(sessions: &[Session]) -> String {
    let mut symbols = String::new();

    for session in sessions {
        if let Some(symbol) = format_window_symbol(session) {
            symbols.push_str(symbol);
        }
    }

    if symbols.is_empty() {
        String::new()
    } else {
        format!("{symbols} ")
    }
}

/// Picks the `label` of the first session (in `sessions`' order, i.e. the
/// existing deterministic pane order from `load_window_sessions`) that has
/// one set, or an empty string if none do.
///
/// Deliberately does not concatenate across sessions: a tmux window
/// normally hosts one worktree / one session in practice, and a multi-session
/// window has no well-defined "combined" title.
fn render_window_title(sessions: &[Session]) -> String {
    sessions
        .iter()
        .find_map(|s| s.label.clone())
        .unwrap_or_default()
}

/// Whether a tmux window option must be rewritten: true when the freshly
/// rendered value differs from what tmux currently holds.
///
/// An unset option (`None`) is treated as an empty string, so a window that
/// never hosted a Claude Code session does not get a redundant write.
fn tmux_option_changed(current: Option<&str>, rendered: &str) -> bool {
    current.unwrap_or("") != rendered
}

/// Formats a single status symbol for embedding in tmux's window-status.
///
/// Returns `None` only for `Ended` sessions (Claude Code fully exited).
/// `Stopped` and `Paused` are still shown: their panes are alive and the
/// conversation is resumable, which is exactly what the window indicator is
/// for.
///
/// No tmux style markup is emitted. A foreground color here would visually
/// become a *background* color whenever the surrounding context has the
/// `reverse` attribute (a common idiom for `window-status-activity-style`),
/// painting the icon's cell with a color block that breaks out of the rest
/// of the tab. Shape alone (●/◐/○/⏸) carries the status well enough.
fn format_window_symbol(session: &Session) -> Option<&'static str> {
    if session.status == SessionStatus::Ended {
        return None;
    }
    Some(session.display_symbol())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use rstest::rstest;
    use std::path::PathBuf;

    fn session(status: SessionStatus, read_at: Option<chrono::DateTime<Utc>>) -> Session {
        Session {
            session_id: "test-123".to_string(),
            cwd: PathBuf::from("/tmp/test"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: std::collections::BTreeSet::new(),
            pending_agent_task_ids: std::collections::BTreeSet::new(),
            read_at,
            sweep_signaled: false,
        }
    }

    fn render(statuses: &[SessionStatus]) -> String {
        let sessions: Vec<Session> = statuses.iter().copied().map(|s| session(s, None)).collect();
        render_window_status(&sessions)
    }

    #[rstest]
    #[case::running(SessionStatus::Running, None, Some("\u{25cf}"))]
    #[case::waiting(SessionStatus::WaitingInput, None, Some("\u{25d0}"))]
    #[case::stopped_unread(SessionStatus::Stopped, None, Some("\u{2731}"))]
    #[case::stopped_read(SessionStatus::Stopped, Some(Utc::now()), Some("\u{25cb}"))]
    #[case::paused(SessionStatus::Paused, None, Some("\u{23f8}"))]
    #[case::ended(SessionStatus::Ended, None, None)]
    fn test_format_window_symbol(
        #[case] status: SessionStatus,
        #[case] read_at: Option<chrono::DateTime<Utc>>,
        #[case] expected: Option<&str>,
    ) {
        assert_eq!(format_window_symbol(&session(status, read_at)), expected);
    }

    #[rstest]
    #[case::empty(&[], "")]
    #[case::single_running(&[SessionStatus::Running], "\u{25cf} ")]
    #[case::paused(&[SessionStatus::Paused], "\u{23f8} ")]
    #[case::running_waiting_stopped(
        &[SessionStatus::Running, SessionStatus::WaitingInput, SessionStatus::Stopped],
        "\u{25cf}\u{25d0}\u{2731} "
    )]
    #[case::skips_ended(
        &[SessionStatus::Ended, SessionStatus::Running],
        "\u{25cf} "
    )]
    #[case::only_ended(&[SessionStatus::Ended], "")]
    fn test_render_window_status(#[case] statuses: &[SessionStatus], #[case] expected: &str) {
        assert_eq!(render(statuses), expected);
    }

    #[rstest]
    #[case::unset_and_empty(None, "", false)]
    #[case::unset_and_nonempty(None, "\u{25cf} ", true)]
    #[case::unchanged(Some("\u{25cf} "), "\u{25cf} ", false)]
    #[case::status_changed(Some("\u{25cf} "), "\u{25d0} ", true)]
    #[case::cleared(Some("\u{25cf} "), "", true)]
    fn test_tmux_option_changed(
        #[case] current: Option<&str>,
        #[case] rendered: &str,
        #[case] expected: bool,
    ) {
        assert_eq!(tmux_option_changed(current, rendered), expected);
    }

    fn session_with_label(label: Option<&str>) -> Session {
        let mut s = session(SessionStatus::Running, None);
        s.label = label.map(str::to_string);
        s
    }

    #[rstest]
    #[case::no_label_anywhere(vec![None, None], "")]
    #[case::first_session_has_label(vec![Some("Fix login bug"), None], "Fix login bug")]
    #[case::first_none_second_has_label(vec![None, Some("Second title")], "Second title")]
    #[case::empty_label_string_counts_as_has_label(vec![Some(""), Some("Ignored")], "")]
    fn test_render_window_title(#[case] labels: Vec<Option<&str>>, #[case] expected: &str) {
        let sessions: Vec<Session> = labels.into_iter().map(session_with_label).collect();
        assert_eq!(render_window_title(&sessions), expected);
    }
}
