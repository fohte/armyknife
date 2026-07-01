mod app;
mod clean_progress;
mod clean_view;
mod event;
mod pr_fetch;
mod session_tree;
mod ui;
mod worktree_session_children;
mod worktree_view;

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::DefaultTerminal;

use self::app::{App, AppMode, View};
use self::event::{AppEvent, EventHandler, KeyEvent, SessionChange, SessionChangeType};
use self::worktree_view::WorktreeMode;
use crate::commands::cc::types::SessionStatus;
use crate::infra::tmux;
use crate::shared::command;

/// Runs the TUI application.
pub fn run() -> Result<()> {
    let mut terminal = ratatui::init();
    let result = run_app(&mut terminal);
    ratatui::restore();
    result
}

/// Side effects requested by key handlers that need access to the event
/// handler (to spawn background work). Returning these from the pure
/// handlers keeps them testable without dragging in real async / IO.
#[derive(Debug, Default, PartialEq, Eq)]
struct KeyEffects {
    /// User pressed `c`: kick off the PR fetch for the clean view.
    request_clean_pr_fetch: bool,
    /// User confirmed `y` in the clean view: spawn the detached child
    /// for these paths and start tailing its log.
    spawn_detached_clean: Option<Vec<PathBuf>>,
    /// User pressed `p`: open the selected session's JSONL in
    /// `claude-history` after suspending the TUI.
    preview_session_path: Option<PathBuf>,
}

impl KeyEffects {
    fn merge(&mut self, other: KeyEffects) {
        if other.request_clean_pr_fetch {
            self.request_clean_pr_fetch = true;
        }
        if other.spawn_detached_clean.is_some() {
            self.spawn_detached_clean = other.spawn_detached_clean;
        }
        if other.preview_session_path.is_some() {
            self.preview_session_path = other.preview_session_path;
        }
    }
}

/// Maximum events to drain per iteration to prevent starvation under sustained load.
const MAX_DRAIN_PER_ITERATION: usize = 100;

/// Main application loop.
///
/// Uses an event-drain strategy to prevent queue buildup:
/// 1. Block-wait for the first event
/// 2. Drain remaining queued events (non-blocking, up to MAX_DRAIN_PER_ITERATION)
/// 3. Key events are processed immediately during drain
/// 4. SessionsChanged events are merged (deduplicated by session_id)
/// 5. The merged reload + render happens once per iteration
fn run_app(terminal: &mut DefaultTerminal) -> Result<()> {
    let mut app = App::new()?;
    let event_handler = EventHandler::new()?;

    loop {
        let unresolved = app.claim_unresolved_label_cwds();
        if !unresolved.is_empty() {
            event_handler.start_session_labels_resolve(unresolved);
        }

        terminal.draw(|frame| ui::render(frame, &mut app))?;

        let first_event = event_handler.next()?;
        let mut needs_full_reload = false;
        let mut change_map: HashMap<String, SessionChangeType> = HashMap::new();
        let mut effects = KeyEffects::default();

        // Process first event + drain queued events (bounded to prevent starvation)
        for event in std::iter::once(first_event)
            .chain(std::iter::from_fn(|| event_handler.try_next()))
            .take(MAX_DRAIN_PER_ITERATION)
        {
            match event {
                AppEvent::Key(k) => effects.merge(handle_key_event(&mut app, k)),
                AppEvent::SessionsChanged(Some(changes)) => {
                    for c in changes {
                        change_map.insert(c.session_id, c.change_type);
                    }
                }
                AppEvent::SessionsChanged(None) => needs_full_reload = true,
                AppEvent::Tick => {}
                AppEvent::WorktreesLoaded(Ok(rows)) => {
                    app.set_worktrees(rows);
                    // If the user opened the clean view before discovery
                    // finished, seed it now and kick off the PR fetch.
                    if app.seed_clean_view_if_pending() {
                        effects.request_clean_pr_fetch = true;
                    }
                }
                AppEvent::WorktreesLoaded(Err(err)) => {
                    app.set_worktrees_failed(err);
                }
                AppEvent::SessionLabelsResolved(results) => {
                    app.apply_resolved_labels(results);
                }
                AppEvent::CleanPrFetched(Ok(rows)) => {
                    app.apply_clean_pr_results(rows);
                }
                AppEvent::CleanPrFetched(Err(err)) => {
                    app.set_clean_failed(err);
                }
                AppEvent::CleanLogEvents(events) => {
                    app.apply_clean_log_events(&events);
                }
            }
        }

        if effects.request_clean_pr_fetch {
            let rows = app.worktree_rows_snapshot();
            let sessions = app.sessions.clone();
            event_handler.start_clean_pr_fetch(rows, sessions);
        }
        if let Some(path) = effects.preview_session_path {
            match preview_session(terminal, &path) {
                Ok(()) => {}
                Err(PreviewError::Viewer(e)) => {
                    app.set_error(format!("Failed to preview session: {e}"));
                }
                // A terminal that will not restore is not something we can
                // paper over — bail out so `ratatui::restore()` runs and
                // the shell gets a chance to clean up.
                Err(PreviewError::Fatal(e)) => return Err(e),
            }
        }
        if let Some(paths) = effects.spawn_detached_clean {
            match clean_progress::spawn_detached_clean(&paths) {
                Ok(run_id) => {
                    app.clean_progress = Some(clean_progress::CleanProgress::new(run_id.clone()));
                    if let Some(log_path) = clean_progress::live_log_path() {
                        event_handler.start_clean_tail(log_path, run_id);
                    }
                }
                Err(e) => {
                    app.set_error(format!("Failed to spawn cleanup: {e}"));
                }
            }
        }

        // Apply merged session changes in a single reload
        let mut sessions_changed = false;
        if let Some(merged) = merge_session_changes(change_map, needs_full_reload) {
            if !merged.is_empty() {
                app.reload_sessions(Some(&merged))?;
                sessions_changed = true;
            }
        } else {
            app.reload_sessions(None)?;
            sessions_changed = true;
        }
        if sessions_changed {
            // Keep the worktree overlay (session count + active marker) in
            // sync without re-running git discovery.
            let snapshot = app.sessions.clone();
            app.worktree_view.refresh_session_overlay(&snapshot);
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

/// Merges session changes by deduplicating on session_id, keeping the last change_type.
/// Returns `None` if `needs_full_reload` is true (caller should do a full reload).
/// Returns `Some(vec)` with deduplicated changes, or `Some(empty)` if no changes.
fn merge_session_changes(
    change_map: HashMap<String, SessionChangeType>,
    needs_full_reload: bool,
) -> Option<Vec<SessionChange>> {
    if needs_full_reload {
        return None;
    }
    Some(
        change_map
            .into_iter()
            .map(|(session_id, change_type)| SessionChange {
                session_id,
                change_type,
            })
            .collect(),
    )
}

/// Focuses on the selected session's tmux pane.
fn focus_selected_session(app: &mut App) {
    if let Some(session) = app.selected_session()
        && let Some(ref tmux_info) = session.tmux_info
        && let Err(e) = tmux::focus_pane(&tmux_info.pane_id)
    {
        app.set_error(format!("Failed to focus tmux pane: {e}"));
    }
}

/// Preview a session's JSONL with `claude-history`.
fn preview_session(
    terminal: &mut DefaultTerminal,
    jsonl_path: &Path,
) -> std::result::Result<(), PreviewError> {
    let suspend_err = suspend_terminal().err();
    // Even if suspend partially failed (raw mode disabled but alt screen
    // still active), we must still try to restore — the alternative is
    // leaving the TUI in a broken state.
    let status = if suspend_err.is_none() {
        Some(command::new("claude-history").arg(jsonl_path).status())
    } else {
        None
    };
    let restore_err = resume_terminal(terminal).err();
    combine_preview_result(suspend_err, status, restore_err)
}

/// Categorized failure from `preview_session`. `Fatal` means the terminal
/// itself is in a bad state (suspend or restore failed) — the caller must
/// bail so `ratatui::restore()` runs. `Viewer` means only the child failed,
/// so the TUI can keep going.
#[derive(Debug)]
enum PreviewError {
    Fatal(anyhow::Error),
    Viewer(anyhow::Error),
}

impl std::fmt::Display for PreviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PreviewError::Fatal(e) | PreviewError::Viewer(e) => write!(f, "{e:#}"),
        }
    }
}

fn combine_preview_result(
    suspend_err: Option<io::Error>,
    status: Option<io::Result<std::process::ExitStatus>>,
    restore_err: Option<io::Error>,
) -> std::result::Result<(), PreviewError> {
    // A non-zero exit is a viewer failure the user should see, not just a
    // spawn failure — collapse both into the same "child error" slot.
    let child_err = status.and_then(|r| match r {
        Ok(st) if !st.success() => {
            Some(io::Error::other(format!("claude-history exited with {st}")))
        }
        Ok(_) => None,
        Err(e) => Some(e),
    });
    match (suspend_err, child_err, restore_err) {
        (None, None, None) => Ok(()),
        (Some(e), _, None) => Err(PreviewError::Fatal(
            anyhow::Error::from(e).context("failed to leave alt screen"),
        )),
        // Restore is the state that governs the next frame, so its error
        // surfaces first with the suspend error folded in.
        (Some(se), _, Some(re)) => Err(PreviewError::Fatal(anyhow::Error::from(re).context(
            format!("failed to restore terminal (suspend also failed: {se})"),
        ))),
        (None, None, Some(re)) => Err(PreviewError::Fatal(
            anyhow::Error::from(re).context("failed to restore terminal"),
        )),
        (None, Some(e), None) => Err(PreviewError::Viewer(
            anyhow::Error::from(e).context("failed to run claude-history"),
        )),
        // Restore failure surfaces first — a broken terminal is more urgent
        // than a missing viewer — with the child error folded in.
        (None, Some(ce), Some(re)) => Err(PreviewError::Fatal(anyhow::Error::from(re).context(
            format!("failed to restore terminal (child also failed: {ce})"),
        ))),
    }
}

fn suspend_terminal() -> io::Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}

fn resume_terminal(terminal: &mut DefaultTerminal) -> io::Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    terminal.clear()?;
    Ok(())
}

const SHELL_COMMANDS: &[&str] = &["zsh", "bash", "fish", "sh", "dash"];

fn resume_selected_session(app: &mut App) {
    let Some(session) = app.selected_session() else {
        return;
    };
    let Some(ref tmux_info) = session.tmux_info else {
        app.set_error("No tmux pane for this session".to_string());
        return;
    };
    if session.status != SessionStatus::Paused {
        app.set_error("Session is not paused".to_string());
        return;
    }
    let pane_id = &tmux_info.pane_id;

    // Only respawn if the pane is sitting at a shell prompt. If the user
    // started another program in the pane we must not kill it silently.
    match tmux::get_pane_current_command(pane_id) {
        Some(cmd) if SHELL_COMMANDS.iter().any(|s| cmd == *s) => {}
        Some(cmd) => {
            app.set_error(format!("Pane is running `{cmd}`, cannot resume"));
            return;
        }
        None => {
            app.set_error("Cannot read pane state".to_string());
            return;
        }
    }

    // Wrap the resume command in the user's login shell so that when claude
    // exits normally, control returns to a shell prompt instead of tmux
    // closing the pane (respawn-pane replaces the pane's root process).
    //
    // `-i` is required on the outer shell: `a cc resume` looks up `claude`
    // in $PATH via `find_command_path`, and many users only extend $PATH
    // in their interactive rc file (e.g. `.zshrc`). Running without `-i`
    // would inherit tmux's pre-rc $PATH and fail to locate `claude`.
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_else(|| "a".to_string());
    let Ok(inner) = shlex::try_join([exe.as_str(), "cc", "resume"]) else {
        app.set_error("Failed to build resume command".to_string());
        return;
    };
    let Ok(exec_shell) = shlex::try_join([shell.as_str(), "-i"]) else {
        app.set_error("Failed to build shell exec command".to_string());
        return;
    };
    let script = format!("{inner}; exec {exec_shell}");
    let Ok(wrapped) = shlex::try_join([shell.as_str(), "-i", "-c", &script]) else {
        app.set_error("Failed to build wrapped command".to_string());
        return;
    };
    if let Err(e) = tmux::respawn_pane(pane_id, &wrapped) {
        app.set_error(format!("Failed to respawn pane: {e}"));
        return;
    }
    if let Err(e) = tmux::focus_pane(pane_id) {
        app.set_error(format!("Failed to focus pane: {e}"));
    }
}

/// Handles key events in Search mode.
fn handle_search_key_event(app: &mut App, key: KeyEvent) {
    match (key.code, key.modifiers) {
        // Cancel search
        (KeyCode::Esc, _) => {
            app.cancel_search();
        }

        // Confirm search and focus on selected session
        (KeyCode::Enter, _) => {
            app.confirm_search();
            focus_selected_session(app);
        }

        // Navigation within filtered results (Ctrl+n/p or arrow keys only)
        (KeyCode::Char('n'), KeyModifiers::CONTROL) | (KeyCode::Down, _) => {
            app.select_next();
        }
        (KeyCode::Char('p'), KeyModifiers::CONTROL) | (KeyCode::Up, _) => {
            app.select_previous();
        }

        // Clear entire search query
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
            app.update_search_query(String::new());
        }

        // Delete last word
        (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
            let query = app.search_query.clone();
            let trimmed = query.trim_end();
            let new_query = if let Some(pos) = trimmed.rfind(char::is_whitespace) {
                trimmed[..=pos].to_string()
            } else {
                String::new()
            };
            app.update_search_query(new_query);
        }

        // Delete character
        (KeyCode::Backspace, _) => {
            let mut query = app.search_query.clone();
            query.pop();
            app.update_search_query(query);
        }

        // Add character to search query (including j/k)
        (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            let mut query = app.search_query.clone();
            query.push(c);
            app.update_search_query(query);
        }

        _ => {}
    }
}

/// Handles key events in Normal mode.
fn handle_normal_key_event(app: &mut App, key: KeyEvent) {
    // Clear error message on any key press
    app.clear_error();

    match (key.code, key.modifiers) {
        // Enter search mode
        (KeyCode::Char('/'), _) => {
            app.enter_search_mode();
        }

        // Clear filter or quit
        (KeyCode::Esc, _) => {
            if app.has_filter() {
                app.clear_filter();
            } else {
                app.quit();
            }
        }

        // Quit
        (KeyCode::Char('q'), KeyModifiers::NONE) => {
            app.quit();
        }

        // Navigation
        (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => {
            app.select_next();
        }
        (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => {
            app.select_previous();
        }

        // Focus on selected session's tmux pane
        (KeyCode::Enter, _) | (KeyCode::Char('f'), KeyModifiers::NONE) => {
            focus_selected_session(app);
        }

        // Resume a paused session
        (KeyCode::Char('r'), KeyModifiers::NONE) => {
            resume_selected_session(app);
        }

        // Delete selected session (with confirmation)
        (KeyCode::Char('d'), KeyModifiers::NONE) => {
            app.request_delete();
        }

        // Status filters (toggle). Use Ctrl-prefixed bindings so that plain
        // letters (`r`, `s`, `w`) remain available for other actions such as
        // resuming a paused session.
        (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
            app.toggle_status_filter(SessionStatus::WaitingInput);
        }
        (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
            app.toggle_status_filter(SessionStatus::Stopped);
        }
        (KeyCode::Char('r'), KeyModifiers::CONTROL) => {
            app.toggle_status_filter(SessionStatus::Running);
        }
        (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
            app.toggle_status_filter(SessionStatus::Paused);
        }

        // Quick select (1-9)
        (KeyCode::Char(c), KeyModifiers::NONE) if c.is_ascii_digit() && c != '0' => {
            let num = c.to_digit(10).unwrap_or(0) as usize;
            app.select_by_number(num);
        }

        _ => {}
    }
}

/// Handles key events in Confirm mode.
fn handle_confirm_key_event(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('y') => {
            if let Err(e) = app.confirm_delete() {
                app.set_error(format!("Failed to delete session: {e}"));
            }
        }
        KeyCode::Char('n') | KeyCode::Esc => {
            app.cancel_confirm();
        }
        _ => {}
    }
}

/// Handles key events in ConfirmWorktreeCleanup mode.
fn handle_confirm_worktree_cleanup_key_event(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('y') => {
            if let Err(e) = app.confirm_worktree_cleanup() {
                app.set_error(format!("Failed to clean up worktree: {e}"));
            }
        }
        KeyCode::Char('n') | KeyCode::Esc => {
            app.cancel_confirm();
        }
        _ => {}
    }
}

/// Handles key events based on the current view and sub-mode.
fn handle_key_event(app: &mut App, key: KeyEvent) -> KeyEffects {
    // One-shot banners: any key press dismisses them so they do not
    // linger over later renders.
    if app.clean_progress.as_ref().is_some_and(|p| p.done) {
        app.clear_clean_progress();
    }

    match app.view {
        View::Session => handle_session_view_key_event(app, key),
        View::Worktree => handle_worktree_view_key_event(app, key),
        View::Clean => handle_clean_view_key_event(app, key),
    }
}

fn handle_session_view_key_event(app: &mut App, key: KeyEvent) -> KeyEffects {
    // Tab cycles views from any non-text-input session sub-mode.
    if app.mode == AppMode::Normal
        && let (KeyCode::Tab, _) = (key.code, key.modifiers)
    {
        app.cycle_view();
        return KeyEffects::default();
    }
    // `c` from Normal mode enters the clean view from the session list.
    if app.mode == AppMode::Normal
        && let (KeyCode::Char('c'), KeyModifiers::NONE) = (key.code, key.modifiers)
    {
        let seeded = app.enter_clean_view();
        return KeyEffects {
            request_clean_pr_fetch: seeded,
            ..Default::default()
        };
    }
    // `p` previews the selected session's JSONL. No-op with no selection
    // or a session that has never emitted a transcript (JSONL is what
    // the viewer reads — nothing to open without it).
    if app.mode == AppMode::Normal
        && let (KeyCode::Char('p'), KeyModifiers::NONE) = (key.code, key.modifiers)
    {
        app.clear_error();
        let path = app
            .selected_session()
            .and_then(|s| s.transcript_path.clone());
        return KeyEffects {
            preview_session_path: path,
            ..Default::default()
        };
    }

    match app.mode {
        AppMode::Normal => handle_normal_key_event(app, key),
        AppMode::Search => handle_search_key_event(app, key),
        AppMode::Confirm { .. } => handle_confirm_key_event(app, key),
        AppMode::ConfirmWorktreeCleanup { .. } => {
            handle_confirm_worktree_cleanup_key_event(app, key);
        }
    }
    KeyEffects::default()
}

fn handle_worktree_view_key_event(app: &mut App, key: KeyEvent) -> KeyEffects {
    // Sub-mode dispatcher: confirmations have their own keys.
    if let WorktreeMode::Confirm { .. } = app.worktree_view.mode {
        match key.code {
            KeyCode::Char('y') => {
                if let Err(e) = app.worktree_view_confirm_delete() {
                    app.set_error(format!("Failed to delete worktree: {e}"));
                }
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                app.worktree_view_cancel_confirm();
            }
            _ => {}
        }
        return KeyEffects::default();
    }

    app.clear_error();
    match (key.code, key.modifiers) {
        (KeyCode::Tab, _) => app.cycle_view(),
        (KeyCode::Char('q'), KeyModifiers::NONE) => app.quit(),
        (KeyCode::Esc, _) => app.quit(),
        (KeyCode::Char('c'), KeyModifiers::NONE) => {
            let seeded = app.enter_clean_view();
            return KeyEffects {
                request_clean_pr_fetch: seeded,
                ..Default::default()
            };
        }
        (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => {
            app.worktree_view.select_next();
        }
        (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => {
            app.worktree_view.select_previous();
        }
        (KeyCode::Enter, _) | (KeyCode::Char('f'), KeyModifiers::NONE) => {
            focus_selected_worktree_session(app);
        }
        (KeyCode::Char('d'), KeyModifiers::NONE) => {
            app.worktree_view_request_delete();
        }
        (KeyCode::Char(c), KeyModifiers::NONE) if c.is_ascii_digit() && c != '0' => {
            if let Some(num) = c.to_digit(10) {
                app.worktree_view.select_by_number(num as usize);
            }
        }
        _ => {}
    }
    KeyEffects::default()
}

/// Handles key events in the clean view (modal-style; Tab is a no-op).
fn handle_clean_view_key_event(app: &mut App, key: KeyEvent) -> KeyEffects {
    app.clear_error();
    match (key.code, key.modifiers) {
        // Cancel: return to the previous view without acting.
        (KeyCode::Esc, _)
        | (KeyCode::Char('n'), KeyModifiers::NONE)
        | (KeyCode::Char('q'), KeyModifiers::NONE) => {
            app.exit_clean_view();
        }
        // Confirm: spawn detached child with all To-delete paths and
        // return to the previous view so progress can show in the
        // bottom bar of session / worktree view.
        (KeyCode::Char('y'), KeyModifiers::NONE) => {
            let paths = app.clean_view.to_delete_paths();
            if paths.is_empty() {
                // Nothing to do — quietly fall back.
                app.exit_clean_view();
                return KeyEffects::default();
            }
            app.exit_clean_view();
            return KeyEffects {
                spawn_detached_clean: Some(paths),
                ..Default::default()
            };
        }
        (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => {
            app.clean_view.select_next();
        }
        (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => {
            app.clean_view.select_previous();
        }
        (KeyCode::Enter, _) => {
            if let Some(child) = app.clean_view.selected_session_child() {
                focus_session_child(app, &child);
            } else {
                app.clean_view.toggle_selected_section();
            }
        }
        _ => {}
    }
    KeyEffects::default()
}

fn focus_selected_worktree_session(app: &mut App) {
    if let Some(child) = app.worktree_view.selected_session_child() {
        focus_session_child(app, &child);
        return;
    }
    let pane_id = match app.worktree_view_focus_session() {
        Some(s) => match s.tmux_info.as_ref() {
            Some(t) => t.pane_id.clone(),
            None => {
                app.set_error("No tmux pane for this session".to_string());
                return;
            }
        },
        None => {
            app.set_error("No sessions in this worktree to focus".to_string());
            return;
        }
    };
    if let Err(e) = tmux::focus_pane(&pane_id) {
        app.set_error(format!("Failed to focus tmux pane: {e}"));
    }
}

fn focus_session_child(app: &mut App, child: &self::worktree_session_children::SessionChild) {
    let Some(pane_id) = child.pane_id.as_deref() else {
        app.set_error("No tmux pane for this session".to_string());
        return;
    };
    if let Err(e) = tmux::focus_pane(pane_id) {
        app.set_error(format!("Failed to focus tmux pane: {e}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::tui::app::AppMode;
    use crate::commands::cc::types::{Session, SessionStatus};
    use chrono::Utc;
    use rstest::{fixture, rstest};
    use std::path::PathBuf;

    fn create_test_app_with_sessions(count: usize) -> App {
        let sessions: Vec<Session> = (0..count)
            .map(|i| Session {
                session_id: format!("session-{}", i),
                cwd: PathBuf::from(format!("/project/{}", i)),
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
                pending_bg_task_ids: std::collections::BTreeSet::new(),
                read_at: None,
            })
            .collect();

        App::with_sessions(sessions)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn key_ctrl(c: char) -> KeyEvent {
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::CONTROL,
        }
    }

    fn type_string(app: &mut App, s: &str) {
        for c in s.chars() {
            handle_key_event(app, key(KeyCode::Char(c)));
        }
    }

    #[rstest]
    #[case::q(KeyCode::Char('q'))]
    #[case::esc(KeyCode::Esc)]
    fn test_handle_key_quit(#[case] code: KeyCode) {
        let mut app = create_test_app_with_sessions(1);
        handle_key_event(&mut app, key(code));
        assert!(app.should_quit);
    }

    #[rstest]
    #[case::j(KeyCode::Char('j'), Some(0), Some(1))]
    #[case::k(KeyCode::Char('k'), Some(2), Some(1))]
    #[case::down(KeyCode::Down, Some(0), Some(1))]
    #[case::up(KeyCode::Up, Some(1), Some(0))]
    fn test_handle_key_navigation(
        #[case] code: KeyCode,
        #[case] initial: Option<usize>,
        #[case] expected: Option<usize>,
    ) {
        let mut app = create_test_app_with_sessions(3);
        app.list_state.select(initial);
        handle_key_event(&mut app, key(code));
        assert_eq!(app.list_state.selected(), expected);
    }

    #[rstest]
    #[case::select_3('3', Some(0), Some(2))]
    #[case::select_1('1', Some(2), Some(0))]
    #[case::zero_ignored('0', Some(2), Some(2))]
    fn test_handle_key_quick_select(
        #[case] c: char,
        #[case] initial: Option<usize>,
        #[case] expected: Option<usize>,
    ) {
        let mut app = create_test_app_with_sessions(5);
        app.list_state.select(initial);
        handle_key_event(&mut app, key(KeyCode::Char(c)));
        assert_eq!(app.list_state.selected(), expected);
    }

    // =========================================================================
    // Search mode tests
    // =========================================================================

    #[test]
    fn test_enter_search_mode_with_slash() {
        let mut app = create_test_app_with_sessions(3);
        assert_eq!(app.mode, AppMode::Normal);

        handle_key_event(&mut app, key(KeyCode::Char('/')));
        assert_eq!(app.mode, AppMode::Search);
    }

    #[test]
    fn test_search_mode_typing() {
        let mut app = create_test_app_with_sessions(3);
        handle_key_event(&mut app, key(KeyCode::Char('/')));
        type_string(&mut app, "test");
        assert_eq!(app.search_query, "test");
    }

    #[test]
    fn test_search_mode_jk_are_typed_not_navigation() {
        let mut app = create_test_app_with_sessions(3);
        handle_key_event(&mut app, key(KeyCode::Char('/')));
        type_string(&mut app, "jk");
        // j and k should be typed as characters, not used for navigation
        assert_eq!(app.search_query, "jk");
    }

    #[test]
    fn test_search_mode_ctrl_n_p_for_navigation() {
        let mut app = create_test_app_with_sessions(3);
        handle_key_event(&mut app, key(KeyCode::Char('/')));

        // Ctrl+n should move down
        handle_key_event(&mut app, key_ctrl('n'));
        assert_eq!(app.list_state.selected(), Some(1));

        // Ctrl+p should move up
        handle_key_event(&mut app, key_ctrl('p'));
        assert_eq!(app.list_state.selected(), Some(0));
    }

    #[test]
    fn test_search_mode_backspace() {
        let mut app = create_test_app_with_sessions(3);
        handle_key_event(&mut app, key(KeyCode::Char('/')));
        type_string(&mut app, "tes");
        handle_key_event(&mut app, key(KeyCode::Backspace));
        assert_eq!(app.search_query, "te");
    }

    #[test]
    fn test_search_mode_ctrl_u_clears_query() {
        let mut app = create_test_app_with_sessions(3);
        handle_key_event(&mut app, key(KeyCode::Char('/')));
        type_string(&mut app, "test");
        handle_key_event(&mut app, key_ctrl('u'));
        assert_eq!(app.search_query, "");
    }

    #[test]
    fn test_search_mode_ctrl_w_deletes_word() {
        let mut app = create_test_app_with_sessions(3);
        handle_key_event(&mut app, key(KeyCode::Char('/')));
        type_string(&mut app, "hello world");
        handle_key_event(&mut app, key_ctrl('w'));
        assert_eq!(app.search_query, "hello ");
    }

    #[test]
    fn test_search_mode_confirm() {
        let mut app = create_test_app_with_sessions(3);
        handle_key_event(&mut app, key(KeyCode::Char('/')));
        type_string(&mut app, "0");
        handle_key_event(&mut app, key(KeyCode::Enter));

        assert_eq!(app.mode, AppMode::Normal);
        assert_eq!(app.confirmed_query, "0");
        assert!(app.has_filter());
    }

    #[test]
    fn test_search_mode_cancel() {
        let mut app = create_test_app_with_sessions(3);
        handle_key_event(&mut app, key(KeyCode::Char('/')));
        type_string(&mut app, "te");
        handle_key_event(&mut app, key(KeyCode::Esc));

        assert_eq!(app.mode, AppMode::Normal);
        assert!(!app.has_filter());
    }

    #[test]
    fn test_esc_clears_filter_in_normal_mode() {
        let mut app = create_test_app_with_sessions(3);

        handle_key_event(&mut app, key(KeyCode::Char('/')));
        type_string(&mut app, "0");
        handle_key_event(&mut app, key(KeyCode::Enter));
        assert!(app.has_filter());

        handle_key_event(&mut app, key(KeyCode::Esc));
        assert!(!app.has_filter());
        assert!(!app.should_quit);
    }

    #[test]
    fn test_esc_quits_when_no_filter() {
        let mut app = create_test_app_with_sessions(3);
        assert!(!app.has_filter());

        handle_key_event(&mut app, key(KeyCode::Esc));
        assert!(app.should_quit);
    }

    // =========================================================================
    // Preview key binding tests
    // =========================================================================

    #[rstest]
    #[case::with_transcript(
        1,
        Some(PathBuf::from("/tmp/session-0.jsonl")),
        key(KeyCode::Char('p')),
        KeyEffects {
            preview_session_path: Some(PathBuf::from("/tmp/session-0.jsonl")),
            ..Default::default()
        },
    )]
    #[case::no_transcript(1, None, key(KeyCode::Char('p')), KeyEffects::default())]
    #[case::no_selection(0, None, key(KeyCode::Char('p')), KeyEffects::default())]
    // Ctrl+p still toggles the Paused filter — plain `p` must not shadow it.
    #[case::ctrl_p_filter(
        1,
        Some(PathBuf::from("/tmp/x.jsonl")),
        key_ctrl('p'),
        KeyEffects::default()
    )]
    fn test_preview_key_effects(
        #[case] session_count: usize,
        #[case] transcript_path: Option<PathBuf>,
        #[case] key: KeyEvent,
        #[case] expected: KeyEffects,
    ) {
        let mut app = create_test_app_with_sessions(session_count);
        if let Some(path) = transcript_path
            && !app.sessions.is_empty()
        {
            app.sessions[0].transcript_path = Some(path);
        }
        assert_eq!(handle_key_event(&mut app, key), expected);
    }

    #[test]
    fn test_ctrl_p_toggles_paused_filter_even_with_p_binding() {
        // Regression guard: plain `p` intercepts before the Ctrl+p filter
        // path, so verify Ctrl+p still reaches it end-to-end.
        let mut app = create_test_app_with_sessions(1);
        handle_key_event(&mut app, key_ctrl('p'));
        assert_eq!(app.status_filter, Some(SessionStatus::Paused));
    }

    // =========================================================================
    // combine_preview_result tests
    // =========================================================================

    fn io_err(msg: &str) -> io::Error {
        io::Error::other(msg)
    }

    fn exit_ok() -> io::Result<std::process::ExitStatus> {
        exit_status(0)
    }

    fn exit_nonzero() -> io::Result<std::process::ExitStatus> {
        // On unix, from_raw takes a wait(2) status; 1 << 8 == exit code 1.
        exit_status(1 << 8)
    }

    fn exit_status(raw: i32) -> io::Result<std::process::ExitStatus> {
        // The repository is Unix-only (tmux + Unix APIs throughout), so no
        // Windows path.
        use std::os::unix::process::ExitStatusExt;
        Ok(std::process::ExitStatus::from_raw(raw))
    }

    /// Categorized outcome for whole-value equality on `combine_preview_result`.
    /// The tag identifies whether the caller should treat the failure as fatal
    /// (terminal broken) or viewer-only.
    #[derive(Debug, PartialEq, Eq)]
    enum ExpectedOutcome {
        Ok,
        Fatal(String),
        Viewer(String),
    }

    fn categorize(res: std::result::Result<(), PreviewError>) -> ExpectedOutcome {
        match res {
            Ok(()) => ExpectedOutcome::Ok,
            Err(PreviewError::Fatal(e)) => ExpectedOutcome::Fatal(format!("{e:#}")),
            Err(PreviewError::Viewer(e)) => ExpectedOutcome::Viewer(format!("{e:#}")),
        }
    }

    fn fatal(s: &str) -> ExpectedOutcome {
        ExpectedOutcome::Fatal(s.to_string())
    }

    fn viewer(s: &str) -> ExpectedOutcome {
        ExpectedOutcome::Viewer(s.to_string())
    }

    #[rstest]
    #[case::all_ok(None, Some(exit_ok()), None, ExpectedOutcome::Ok)]
    #[case::child_spawn_error(
        None,
        Some(Err(io_err("boom"))),
        None,
        viewer("failed to run claude-history: boom")
    )]
    #[case::child_nonzero_exit(
        None,
        Some(exit_nonzero()),
        None,
        viewer("failed to run claude-history: claude-history exited with exit status: 1")
    )]
    #[case::restore_error(
        None,
        Some(exit_ok()),
        Some(io_err("bad")),
        fatal("failed to restore terminal: bad")
    )]
    // When both child and restore fail the restore error surfaces first —
    // a broken terminal is more urgent than a missing viewer — with the
    // child error folded in as context.
    #[case::child_and_restore_error(
        None,
        Some(Err(io_err("no bin"))),
        Some(io_err("no tty")),
        fatal("failed to restore terminal (child also failed: no bin): no tty")
    )]
    // If suspend fails, callers pass status=None (no spawn happened) but
    // restore is still attempted; the suspend error surfaces as fatal.
    #[case::suspend_error_only(
        Some(io_err("suspend")),
        None,
        None,
        fatal("failed to leave alt screen: suspend")
    )]
    #[case::suspend_and_restore_error(
        Some(io_err("suspend")),
        None,
        Some(io_err("no tty")),
        fatal("failed to restore terminal (suspend also failed: suspend): no tty")
    )]
    fn test_combine_preview_result(
        #[case] suspend_err: Option<io::Error>,
        #[case] status: Option<io::Result<std::process::ExitStatus>>,
        #[case] restore_err: Option<io::Error>,
        #[case] expected: ExpectedOutcome,
    ) {
        assert_eq!(
            categorize(combine_preview_result(suspend_err, status, restore_err)),
            expected,
        );
    }

    // =========================================================================
    // Status filter key binding tests
    // =========================================================================

    #[fixture]
    fn app_with_statuses() -> App {
        let sessions: Vec<Session> = vec![
            Session {
                session_id: "session-running".to_string(),
                cwd: PathBuf::from("/project/running"),
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
                pending_bg_task_ids: std::collections::BTreeSet::new(),
                read_at: None,
            },
            Session {
                session_id: "session-waiting".to_string(),
                cwd: PathBuf::from("/project/waiting"),
                transcript_path: None,
                tty: None,
                tmux_info: None,
                status: SessionStatus::WaitingInput,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                last_message: None,
                current_tool: None,
                label: None,
                ancestor_session_ids: Vec::new(),
                pending_bg_task_ids: std::collections::BTreeSet::new(),
                read_at: None,
            },
            Session {
                session_id: "session-stopped".to_string(),
                cwd: PathBuf::from("/project/stopped"),
                transcript_path: None,
                tty: None,
                tmux_info: None,
                status: SessionStatus::Stopped,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                last_message: None,
                current_tool: None,
                label: None,
                ancestor_session_ids: Vec::new(),
                pending_bg_task_ids: std::collections::BTreeSet::new(),
                read_at: None,
            },
            Session {
                session_id: "session-paused".to_string(),
                cwd: PathBuf::from("/project/paused"),
                transcript_path: None,
                tty: None,
                tmux_info: None,
                status: SessionStatus::Paused,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                last_message: None,
                current_tool: None,
                label: None,
                ancestor_session_ids: Vec::new(),
                pending_bg_task_ids: std::collections::BTreeSet::new(),
                read_at: None,
            },
        ];
        App::with_sessions(sessions)
    }

    #[rstest]
    #[case::ctrl_w_toggles_waiting('w', SessionStatus::WaitingInput)]
    #[case::ctrl_s_toggles_stopped('s', SessionStatus::Stopped)]
    #[case::ctrl_r_toggles_running('r', SessionStatus::Running)]
    #[case::ctrl_p_toggles_paused('p', SessionStatus::Paused)]
    fn test_handle_key_status_filter(
        app_with_statuses: App,
        #[case] c: char,
        #[case] expected_status: SessionStatus,
    ) {
        let mut app = app_with_statuses;
        handle_key_event(&mut app, key_ctrl(c));

        assert_eq!(app.status_filter, Some(expected_status));
        // All filtered sessions should have the expected status
        for session in app.filtered_sessions() {
            assert_eq!(session.status, expected_status);
        }
    }

    #[rstest]
    #[case::plain_w('w')]
    #[case::plain_s('s')]
    #[case::plain_r('r')]
    fn test_plain_wsr_do_not_filter(app_with_statuses: App, #[case] c: char) {
        // Without Ctrl, these keys should not activate a status filter -- they
        // are reserved for future actions (e.g., `r` = resume).
        let mut app = app_with_statuses;
        handle_key_event(&mut app, key(KeyCode::Char(c)));
        assert!(app.status_filter.is_none());
    }

    #[rstest]
    fn test_status_filter_toggle_off(app_with_statuses: App) {
        let mut app = app_with_statuses;

        // Press Ctrl+w to set WaitingInput filter
        handle_key_event(&mut app, key_ctrl('w'));
        assert_eq!(app.status_filter, Some(SessionStatus::WaitingInput));
        assert_eq!(app.filtered_sessions().len(), 1);

        // Press Ctrl+w again to clear the filter
        handle_key_event(&mut app, key_ctrl('w'));
        assert!(app.status_filter.is_none());
        assert_eq!(app.filtered_sessions().len(), 4);
    }

    #[rstest]
    fn test_esc_clears_status_filter(app_with_statuses: App) {
        let mut app = app_with_statuses;

        // Set status filter
        handle_key_event(&mut app, key_ctrl('w'));
        assert!(app.has_filter());

        // Press Esc to clear filter (should not quit because filter is active)
        handle_key_event(&mut app, key(KeyCode::Esc));
        assert!(!app.has_filter());
        assert!(app.status_filter.is_none());
        assert!(!app.should_quit);
    }

    // =========================================================================
    // merge_session_changes tests
    // =========================================================================

    #[test]
    fn test_merge_session_changes_full_reload_returns_none() {
        let map = HashMap::from([("s1".to_string(), SessionChangeType::Modified)]);
        // Even with changes in the map, full reload takes precedence
        assert!(merge_session_changes(map, true).is_none());
    }

    #[test]
    fn test_merge_session_changes_empty_map_no_reload() {
        let map = HashMap::new();
        let result = merge_session_changes(map, false);
        assert_eq!(result.unwrap().len(), 0);
    }

    #[test]
    fn test_merge_session_changes_dedup_same_session() {
        // Simulate: session "s1" was Created then Modified → HashMap keeps last insert
        let mut map = HashMap::new();
        map.insert("s1".to_string(), SessionChangeType::Created);
        map.insert("s1".to_string(), SessionChangeType::Modified);

        let result = merge_session_changes(map, false).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].session_id, "s1");
        assert_eq!(result[0].change_type, SessionChangeType::Modified);
    }

    #[test]
    fn test_merge_session_changes_distinct_sessions() {
        let map = HashMap::from([
            ("s1".to_string(), SessionChangeType::Created),
            ("s2".to_string(), SessionChangeType::Deleted),
            ("s3".to_string(), SessionChangeType::Modified),
        ]);

        let mut result = merge_session_changes(map, false).unwrap();
        assert_eq!(result.len(), 3);

        // Sort by session_id for deterministic verification
        result.sort_by_key(|c| c.session_id.clone());

        assert_eq!(result[0].session_id, "s1");
        assert_eq!(result[0].change_type, SessionChangeType::Created);
        assert_eq!(result[1].session_id, "s2");
        assert_eq!(result[1].change_type, SessionChangeType::Deleted);
        assert_eq!(result[2].session_id, "s3");
        assert_eq!(result[2].change_type, SessionChangeType::Modified);
    }

    #[rstest]
    #[case::created_then_deleted(
        SessionChangeType::Created,
        SessionChangeType::Deleted,
        SessionChangeType::Deleted
    )]
    #[case::modified_then_deleted(
        SessionChangeType::Modified,
        SessionChangeType::Deleted,
        SessionChangeType::Deleted
    )]
    #[case::deleted_then_created(
        SessionChangeType::Deleted,
        SessionChangeType::Created,
        SessionChangeType::Created
    )]
    fn test_merge_session_changes_last_wins(
        #[case] first: SessionChangeType,
        #[case] second: SessionChangeType,
        #[case] expected: SessionChangeType,
    ) {
        let mut map = HashMap::new();
        map.insert("s1".to_string(), first);
        map.insert("s1".to_string(), second);

        let result = merge_session_changes(map, false).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].change_type, expected);
    }

    #[test]
    fn test_merge_session_changes_full_reload_overrides_changes() {
        // Even with many changes, full reload means None
        let map = HashMap::from([
            ("s1".to_string(), SessionChangeType::Modified),
            ("s2".to_string(), SessionChangeType::Created),
            ("s3".to_string(), SessionChangeType::Deleted),
        ]);
        assert!(merge_session_changes(map, true).is_none());
    }

    // =========================================================================
    // Delete/confirm mode tests
    // =========================================================================

    #[test]
    fn test_d_key_enters_confirm_mode() {
        let mut app = create_test_app_with_sessions(3);
        let selected_id = app.selected_session().map(|s| s.session_id.clone());
        handle_key_event(&mut app, key(KeyCode::Char('d')));

        assert!(matches!(app.mode, AppMode::Confirm { .. }));
        if let AppMode::Confirm { session_id, .. } = &app.mode {
            assert_eq!(Some(session_id.clone()), selected_id);
        }
    }

    #[test]
    fn test_d_key_no_op_when_no_sessions() {
        let mut app = create_test_app_with_sessions(0);
        handle_key_event(&mut app, key(KeyCode::Char('d')));

        assert_eq!(app.mode, AppMode::Normal);
    }

    #[rstest]
    #[case::n_cancels(KeyCode::Char('n'))]
    #[case::esc_cancels(KeyCode::Esc)]
    fn test_confirm_cancel(#[case] cancel_key: KeyCode) {
        let mut app = create_test_app_with_sessions(3);
        handle_key_event(&mut app, key(KeyCode::Char('d')));
        assert!(matches!(app.mode, AppMode::Confirm { .. }));

        handle_key_event(&mut app, key(cancel_key));
        assert_eq!(app.mode, AppMode::Normal);
        // Sessions should remain unchanged
        assert_eq!(app.sessions.len(), 3);
    }

    #[test]
    fn test_confirm_ignores_unrelated_keys() {
        let mut app = create_test_app_with_sessions(3);
        handle_key_event(&mut app, key(KeyCode::Char('d')));
        assert!(matches!(app.mode, AppMode::Confirm { .. }));

        // Pressing 'j' or other keys should not change mode
        handle_key_event(&mut app, key(KeyCode::Char('j')));
        assert!(matches!(app.mode, AppMode::Confirm { .. }));
    }

    // =========================================================================
    // Two-stage delete tests (worktree cleanup prompt)
    //
    // These verify the state machine only. They do not exercise the actual
    // worktree cleanup path, which depends on external commands (tmux) and
    // is covered by integration tests on `cleanup_worktree_resources`.
    // =========================================================================

    use crate::shared::testing::TestRepo;

    fn session_with_cwd(id: &str, cwd: PathBuf) -> Session {
        Session {
            session_id: id.to_string(),
            cwd,
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
            pending_bg_task_ids: std::collections::BTreeSet::new(),
            read_at: None,
        }
    }

    /// Creates a TestRepo with a single worktree named `feat` and returns it
    /// together with the resolved worktree path. Ownership of `TestRepo` is
    /// returned so the TempDir survives for the lifetime of the test.
    #[fixture]
    fn worktree_feat() -> (TestRepo, PathBuf) {
        let repo = TestRepo::new();
        repo.create_worktree("feat");
        let wt_path = repo.worktree_path("feat");
        (repo, wt_path)
    }

    #[rstest]
    fn test_confirm_delete_with_sibling_in_same_worktree_returns_to_normal(
        worktree_feat: (TestRepo, PathBuf),
    ) {
        // Two sessions share the same worktree. Deleting one must NOT trigger
        // worktree cleanup — the sibling session is still using it.
        let (_repo, wt_path) = worktree_feat;
        let s1 = session_with_cwd("s1", wt_path.join("src"));
        let s2 = session_with_cwd("s2", wt_path.clone());
        let mut app = App::with_sessions(vec![s1, s2]);

        handle_key_event(&mut app, key(KeyCode::Char('d')));
        assert!(matches!(app.mode, AppMode::Confirm { .. }));
        handle_key_event(&mut app, key(KeyCode::Char('y')));

        assert_eq!(
            app.mode,
            AppMode::Normal,
            "sibling session exists; must not prompt for worktree cleanup"
        );
        // One session deleted, one remains
        assert_eq!(app.sessions.len(), 1);
    }

    #[rstest]
    fn test_confirm_delete_last_session_in_worktree_prompts_cleanup(
        worktree_feat: (TestRepo, PathBuf),
    ) {
        // Only session in the worktree. After deletion, user should be asked
        // whether to also remove the worktree itself.
        let (repo, wt_path) = worktree_feat;
        let s1 = session_with_cwd("s1", wt_path);
        let mut app = App::with_sessions(vec![s1]);

        handle_key_event(&mut app, key(KeyCode::Char('d')));
        handle_key_event(&mut app, key(KeyCode::Char('y')));

        match &app.mode {
            AppMode::ConfirmWorktreeCleanup { worktree_root } => {
                assert!(
                    worktree_root.starts_with(repo.path()),
                    "worktree_root should be inside the main repo path"
                );
            }
            other => panic!("expected ConfirmWorktreeCleanup, got {other:?}"),
        }
        assert!(app.sessions.is_empty());
    }

    #[rstest]
    #[case::n_key(KeyCode::Char('n'))]
    #[case::esc_key(KeyCode::Esc)]
    fn test_confirm_worktree_cleanup_cancel_keeps_worktree(
        worktree_feat: (TestRepo, PathBuf),
        #[case] cancel_key: KeyCode,
    ) {
        let (_repo, wt_path) = worktree_feat;
        let s1 = session_with_cwd("s1", wt_path.clone());
        let mut app = App::with_sessions(vec![s1]);

        handle_key_event(&mut app, key(KeyCode::Char('d')));
        handle_key_event(&mut app, key(KeyCode::Char('y')));
        assert!(matches!(app.mode, AppMode::ConfirmWorktreeCleanup { .. }));

        handle_key_event(&mut app, key(cancel_key));
        assert_eq!(app.mode, AppMode::Normal);

        // Worktree directory must still exist on disk.
        assert!(
            wt_path.exists(),
            "worktree directory should remain when cleanup is declined"
        );
    }

    // =========================================================================
    // Clean view key flow tests
    // =========================================================================

    use crate::commands::cc::tui::app::View;
    use crate::commands::cc::tui::clean_view::{CleanRow, CleanSection};

    fn clean_row(section: CleanSection, repo: &str, name: &str, path: &str) -> CleanRow {
        CleanRow {
            repo: repo.to_string(),
            branch: name.to_string(),
            name: name.to_string(),
            path: PathBuf::from(path),
            session_count: 0,
            has_active: false,
            updated_at: None,
            status_label: "PR merged".to_string(),
            pr_merged: section == CleanSection::ToDelete,
            section,
            sessions: Vec::new(),
        }
    }

    fn seed_one_worktree(app: &mut App) {
        app.set_worktrees(vec![crate::commands::cc::tui::worktree_view::WorktreeRow {
            repo: "r1".to_string(),
            branch: "feat-a".to_string(),
            name: "feat-a".to_string(),
            path: PathBuf::from("/tmp/r1/feat-a"),
            session_count: 0,
            has_active: false,
            sessions: Vec::new(),
        }]);
    }

    #[test]
    fn pressing_c_from_session_view_enters_clean_and_requests_pr_fetch() {
        let mut app = create_test_app_with_sessions(1);
        seed_one_worktree(&mut app);
        let effects = handle_key_event(&mut app, key(KeyCode::Char('c')));
        assert_eq!(app.view, View::Clean);
        assert!(effects.request_clean_pr_fetch);
    }

    #[test]
    fn pressing_c_from_worktree_view_enters_clean() {
        let mut app = create_test_app_with_sessions(0);
        seed_one_worktree(&mut app);
        app.view = View::Worktree;
        let effects = handle_key_event(&mut app, key(KeyCode::Char('c')));
        assert_eq!(app.view, View::Clean);
        assert_eq!(app.clean_return_view, View::Worktree);
        assert!(effects.request_clean_pr_fetch);
    }

    #[test]
    fn pressing_c_without_worktree_snapshot_defers_pr_fetch() {
        let mut app = create_test_app_with_sessions(0);
        let effects = handle_key_event(&mut app, key(KeyCode::Char('c')));
        assert_eq!(app.view, View::Clean);
        assert!(!effects.request_clean_pr_fetch);
        assert!(matches!(
            app.clean_view.state,
            crate::commands::cc::tui::clean_view::CleanLoadState::LoadingPr
        ));
    }

    #[test]
    fn seeding_after_empty_worktree_discovery_transitions_to_ready() {
        // Regression: discovery finishing with zero worktrees previously
        // left the view stuck on "Loading worktrees..." because the
        // empty-rows branch never transitioned out of LoadingPr.
        let mut app = create_test_app_with_sessions(0);
        handle_key_event(&mut app, key(KeyCode::Char('c')));
        app.set_worktrees(Vec::new());
        let seeded = app.seed_clean_view_if_pending();
        assert!(!seeded);
        assert!(matches!(
            app.clean_view.state,
            crate::commands::cc::tui::clean_view::CleanLoadState::Ready(_)
        ));
        assert_eq!(
            app.clean_view.pr_fetch,
            crate::commands::cc::tui::clean_view::PrFetchStatus::Done
        );
    }

    #[test]
    fn seeding_after_worktrees_arrive_kicks_off_pr_fetch() {
        let mut app = create_test_app_with_sessions(0);
        handle_key_event(&mut app, key(KeyCode::Char('c')));
        seed_one_worktree(&mut app);
        assert!(app.seed_clean_view_if_pending());
        assert!(matches!(
            app.clean_view.state,
            crate::commands::cc::tui::clean_view::CleanLoadState::Ready(_)
        ));
    }

    #[rstest]
    #[case::esc(KeyCode::Esc)]
    #[case::n(KeyCode::Char('n'))]
    #[case::q(KeyCode::Char('q'))]
    fn clean_view_cancel_returns_to_previous_view(#[case] code: KeyCode) {
        let mut app = create_test_app_with_sessions(0);
        app.view = View::Worktree;
        handle_key_event(&mut app, key(KeyCode::Char('c')));
        assert_eq!(app.view, View::Clean);
        handle_key_event(&mut app, key(code));
        assert_eq!(app.view, View::Worktree);
    }

    #[test]
    fn clean_view_enter_toggles_section() {
        let mut app = create_test_app_with_sessions(0);
        handle_key_event(&mut app, key(KeyCode::Char('c')));
        app.set_clean_rows(vec![
            clean_row(CleanSection::ToDelete, "r1", "feat-a", "/tmp/a"),
            clean_row(CleanSection::Kept, "r1", "fix-b", "/tmp/b"),
        ]);
        let before = app.clean_view.selected_row().expect("row");
        assert_eq!(before.section, CleanSection::ToDelete);

        handle_key_event(&mut app, key(KeyCode::Enter));

        let after = app.clean_view.selected_row().expect("row");
        assert_eq!(after.path, before.path);
        assert_eq!(after.section, CleanSection::Kept);
    }

    #[test]
    fn clean_view_y_with_pending_paths_emits_spawn_effect() {
        let mut app = create_test_app_with_sessions(0);
        handle_key_event(&mut app, key(KeyCode::Char('c')));
        app.set_clean_rows(vec![clean_row(
            CleanSection::ToDelete,
            "r1",
            "feat-a",
            "/tmp/a",
        )]);
        let effects = handle_key_event(&mut app, key(KeyCode::Char('y')));
        assert_eq!(app.view, View::Session); // returned to caller view
        assert_eq!(
            effects.spawn_detached_clean,
            Some(vec![PathBuf::from("/tmp/a")])
        );
    }

    #[test]
    fn clean_view_y_with_empty_to_delete_just_exits() {
        let mut app = create_test_app_with_sessions(0);
        handle_key_event(&mut app, key(KeyCode::Char('c')));
        app.set_clean_rows(vec![clean_row(CleanSection::Kept, "r1", "fix-b", "/tmp/b")]);
        let effects = handle_key_event(&mut app, key(KeyCode::Char('y')));
        assert_eq!(app.view, View::Session);
        assert!(effects.spawn_detached_clean.is_none());
    }

    #[test]
    fn tab_inside_clean_view_is_noop() {
        let mut app = create_test_app_with_sessions(0);
        handle_key_event(&mut app, key(KeyCode::Char('c')));
        let view_before = app.view;
        handle_key_event(&mut app, key(KeyCode::Tab));
        assert_eq!(app.view, view_before);
    }

    #[test]
    fn test_confirm_delete_non_worktree_session_returns_to_normal() {
        // cwd is outside any git repository. No prompt, just plain delete.
        let tmp = tempfile::tempdir().expect("tempdir");
        let s1 = session_with_cwd("s1", tmp.path().to_path_buf());
        let mut app = App::with_sessions(vec![s1]);

        handle_key_event(&mut app, key(KeyCode::Char('d')));
        handle_key_event(&mut app, key(KeyCode::Char('y')));

        assert_eq!(app.mode, AppMode::Normal);
        assert!(app.sessions.is_empty());
    }
}
