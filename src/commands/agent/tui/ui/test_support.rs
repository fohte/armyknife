use std::collections::BTreeSet;
use std::path::PathBuf;

use chrono::{DateTime, Utc};

use crate::commands::agent::tui::app::App;
use crate::commands::agent::tui::worktree::WorktreeRow;
use crate::commands::agent::types::{Engine, Session, SessionStatus};
use crate::shared::config::AgentConfig;

use super::chrome::render_with_time;

pub(super) fn create_test_session(id: &str) -> Session {
    Session {
        session_id: id.to_string(),
        work_type: None,
        work_type_pinned: false,
        crit_urls: Vec::new(),
        pending_human_review_ids: Default::default(),
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
        pending_bg_task_ids: BTreeSet::new(),
        pending_agent_task_ids: BTreeSet::new(),
        pending_permission_agent_ids: BTreeSet::new(),
        pending_permission_request_ids: Default::default(),
        read_at: None,
        sweep_signaled: false,
        agent_status: None,
        engine: Engine::Claude,
    }
}

pub(super) fn wt_row(repo: &str, branch: &str, name: &str, path: &str) -> WorktreeRow {
    WorktreeRow {
        repo: repo.to_string(),
        branch: branch.to_string(),
        name: name.to_string(),
        path: PathBuf::from(path),
        session_count: 0,
    }
}

/// Renders the entire UI to a TestBackend and returns the raw cell buffer,
/// so tests can assert on styles (fg/bg/modifier) that a plain-text
/// comparison can't see.
pub(super) fn render_buffer(
    sessions: &[Session],
    selected_index: Option<usize>,
    now: DateTime<Utc>,
    width: u16,
    height: u16,
) -> ratatui::buffer::Buffer {
    render_buffer_with(sessions, selected_index, now, width, height, |_| {})
}

/// Same as `render_buffer`, but lets the caller mutate the `App` between
/// construction and render.
pub(super) fn render_buffer_with<F>(
    sessions: &[Session],
    selected_index: Option<usize>,
    now: DateTime<Utc>,
    width: u16,
    height: u16,
    setup: F,
) -> ratatui::buffer::Buffer
where
    F: FnOnce(&mut App),
{
    render_buffer_with_agent_config(
        sessions,
        selected_index,
        now,
        width,
        height,
        AgentConfig::default(),
        setup,
    )
}

pub(super) fn render_buffer_with_agent_config<F>(
    sessions: &[Session],
    selected_index: Option<usize>,
    now: DateTime<Utc>,
    width: u16,
    height: u16,
    agent_config: AgentConfig,
    setup: F,
) -> ratatui::buffer::Buffer
where
    F: FnOnce(&mut App),
{
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();

    let mut app = App::with_sessions_and_agent_config(sessions.to_vec(), agent_config);
    app.list_state.select(selected_index);
    setup(&mut app);

    terminal
        .draw(|frame| {
            render_with_time(frame, &mut app, now);
        })
        .unwrap();

    terminal.backend().buffer().clone()
}

/// Renders the entire UI to a TestBackend for testing.
/// Returns the rendered output as a string.
pub(super) fn render_to_string(
    sessions: &[Session],
    selected_index: Option<usize>,
    now: DateTime<Utc>,
    width: u16,
    height: u16,
) -> String {
    render_to_string_with(sessions, selected_index, now, width, height, |_| {})
}

/// Same as `render_to_string`, but lets the caller mutate the `App`
/// between construction and render — useful to flip into the worktree
/// view, inject worktree rows, etc.
pub(super) fn render_to_string_with<F>(
    sessions: &[Session],
    selected_index: Option<usize>,
    now: DateTime<Utc>,
    width: u16,
    height: u16,
    setup: F,
) -> String
where
    F: FnOnce(&mut App),
{
    let buffer = render_buffer_with(sessions, selected_index, now, width, height, setup);
    buffer_to_string(buffer)
}

/// Renders the full UI with injected agent settings, matching App startup's
/// config path without depending on the user's config directory.
pub(super) fn render_to_string_with_agent_config(
    sessions: &[Session],
    selected_index: Option<usize>,
    now: DateTime<Utc>,
    width: u16,
    height: u16,
    agent_config: AgentConfig,
) -> String {
    let buffer = render_buffer_with_agent_config(
        sessions,
        selected_index,
        now,
        width,
        height,
        agent_config,
        |_| {},
    );
    buffer_to_string(buffer)
}

fn buffer_to_string(buffer: ratatui::buffer::Buffer) -> String {
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
