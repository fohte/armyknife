//! Tmux-resurrect integration for Claude Code session restoration.
//!
//! This module provides commands to save and restore Claude Code session IDs
//! when tmux-resurrect saves/restores tmux sessions. Since tmux user options
//! are not automatically preserved by tmux-resurrect, we need to explicitly
//! save them to a state file and restore them after tmux-resurrect completes.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Subcommand, ValueEnum};

use super::pane;
use super::store;
use super::types::{Engine, Session, TMUX_SESSION_OPTION, TMUX_SESSION_OPTION_LEGACY};
use crate::infra::process::ProcessSnapshot;
use crate::infra::tmux;
use crate::shared::cache;
use crate::shared::log::short_run_id;

/// Directory name for storing resurrect state files.
const RESURRECT_STATE_DIR: &str = "resurrect";

/// File name for the resurrect state file.
const RESURRECT_STATE_FILE: &str = "pane_sessions.txt";

/// Session metadata captured for one pane during the pre-save hook.
type SavedPaneSession = (String, u32, u32, String, Vec<String>, Engine);

/// Session metadata restored by pane position from the state file.
type RestoredPaneSession = (String, Vec<String>, Option<Engine>);

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum ResurrectCommands {
    /// Save all pane session IDs (called from tmux-resurrect post-save hook)
    Save(SaveArgs),

    /// Restore pane session IDs (called from tmux-resurrect post-restore hook)
    Restore(RestoreArgs),
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct SaveArgs {}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct RestoreArgs {}

/// Runs the resurrect subcommand.
pub fn run(cmd: &ResurrectCommands) -> Result<()> {
    match cmd {
        ResurrectCommands::Save(args) => run_save(args),
        ResurrectCommands::Restore(args) => run_restore(args),
    }
}

/// Returns the path to the resurrect state file.
fn state_file_path() -> Result<PathBuf> {
    let base_dir =
        cache::base_dir().ok_or_else(|| anyhow::anyhow!("Could not determine cache directory"))?;
    Ok(base_dir
        .join("cc")
        .join(RESURRECT_STATE_DIR)
        .join(RESURRECT_STATE_FILE))
}

/// Parses a state file line into (pane_position, session_id, ancestor_session_ids, engine).
/// Format: "session_name:window_index.pane_index<TAB>session_id<TAB>ancestor_session_ids<TAB>engine".
/// The ancestor and engine columns are optional for backwards compatibility;
/// state files without an engine column resolve it from the current store.
fn parse_state_line(line: &str) -> Option<(&str, &str, Vec<String>, Option<Engine>)> {
    if line.is_empty() {
        return None;
    }
    match line.split('\t').collect::<Vec<_>>().as_slice() {
        [pane_position, session_id] => Some((pane_position, session_id, Vec::new(), None)),
        [pane_position, session_id, ancestor_ids] => Some((
            pane_position,
            session_id,
            parse_ancestor_ids(ancestor_ids),
            None,
        )),
        [pane_position, session_id, ancestor_ids, engine] => Some((
            pane_position,
            session_id,
            parse_ancestor_ids(ancestor_ids),
            Some(Engine::from_str(engine, false).ok()?),
        )),
        _ => None,
    }
}

fn parse_ancestor_ids(s: &str) -> Vec<String> {
    if s.is_empty() {
        Vec::new()
    } else {
        s.split(',').map(String::from).collect()
    }
}

/// Formats a pane position for state file.
/// Format: "session_name:window_index.pane_index"
fn format_pane_position(session_name: &str, window_index: u32, pane_index: u32) -> String {
    format!("{}:{}.{}", session_name, window_index, pane_index)
}

fn format_engine(engine: Engine) -> &'static str {
    match engine {
        Engine::Claude => "claude",
        Engine::Codex => "codex",
    }
}

/// Writes pane sessions to a state file.
fn write_state_file(state_file: &PathBuf, pane_sessions: &[SavedPaneSession]) -> Result<()> {
    // Ensure parent directory exists
    if let Some(parent) = state_file.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
    }

    let mut file = fs::File::create(state_file)
        .with_context(|| format!("Failed to create state file: {}", state_file.display()))?;

    for (session_name, window_index, pane_index, session_id, ancestor_session_ids, engine) in
        pane_sessions
    {
        let pane_position = format_pane_position(session_name, *window_index, *pane_index);
        writeln!(
            file,
            "{}\t{}\t{}\t{}",
            pane_position,
            session_id,
            ancestor_session_ids.join(","),
            format_engine(*engine),
        )?;
    }

    Ok(())
}

/// Reads pane sessions from a state file.
/// Returns a map of pane_position -> (session_id, ancestor_session_ids, engine).
fn read_state_file(state_file: &PathBuf) -> Result<HashMap<String, RestoredPaneSession>> {
    let file = fs::File::open(state_file)
        .with_context(|| format!("Failed to open state file: {}", state_file.display()))?;
    let reader = BufReader::new(file);

    let mut pane_sessions = HashMap::new();
    for line in reader.lines() {
        let line = line?;
        if let Some((pane_position, session_id, ancestor_session_ids, engine)) =
            parse_state_line(&line)
        {
            pane_sessions.insert(
                pane_position.to_string(),
                (session_id.to_string(), ancestor_session_ids, engine),
            );
        }
    }

    Ok(pane_sessions)
}

/// Saves all pane session IDs to the state file.
///
/// Format: session_name:window_index.pane_index<TAB>session_id<TAB>ancestor_session_ids<TAB>engine
/// This format uses pane position (session:window.pane) rather than pane_id
/// because pane_id changes after tmux-resurrect restore. `ancestor_session_ids`
/// and `engine` are read from the store JSON now, at save time, because a tmux server
/// restart can wipe that JSON (see `store::cleanup_stale_sessions`) before
/// restore gets a chance to run.
fn run_save(_args: &SaveArgs) -> Result<()> {
    let run_id = short_run_id();
    let span = tracing::info_span!("agent.resurrect.save", run_id = %run_id);
    let _entered = span.enter();

    let panes = tmux::list_all_panes_with_option(TMUX_SESSION_OPTION, TMUX_SESSION_OPTION_LEGACY);
    let state_file = state_file_path()?;
    let sessions_dir = store::sessions_dir()?;

    tracing::info!(
        event = "agent.resurrect.save.start",
        pane_count = panes.len()
    );

    // Preserve the existing state file when no panes carry a session ID.
    // The tmux server may simply have just restarted and not yet been restored,
    // and the state file is consumed by tmux-resurrect's post-restore hook.
    if panes.is_empty() {
        tracing::info!(
            event = "agent.resurrect.save.skip",
            reason = "no panes with session option"
        );
        return Ok(());
    }

    // Convert panes to the format expected by write_state_file
    let pane_sessions: Vec<_> = panes
        .into_iter()
        .filter_map(|pane| {
            pane.option_value.map(|session_id| {
                let ancestor_session_ids = load_ancestor_session_ids(&sessions_dir, &session_id);
                let engine = load_engine(&sessions_dir, &session_id);
                (
                    pane.session_name,
                    pane.window_index,
                    pane.pane_index,
                    session_id,
                    ancestor_session_ids,
                    engine,
                )
            })
        })
        .collect();

    tracing::info!(
        event = "agent.resurrect.save.summary",
        saved = pane_sessions.len()
    );

    write_state_file(&state_file, &pane_sessions)
}

/// Reads `engine` for `session_id` from the store, defaulting to
/// `Engine::Claude` (matching `Session::engine`'s own default) when the
/// session file is missing or fails to load.
fn load_engine(sessions_dir: &Path, session_id: &str) -> Engine {
    load_session_field(
        sessions_dir,
        session_id,
        "agent.resurrect.save.engine_load_failed",
        |s| s.engine,
    )
}

/// Reads `ancestor_session_ids` for `session_id` from the store, treating a
/// missing session file the same as one with no ancestors. A read failure
/// (e.g. a lock timeout against a session mid-write) is logged rather than
/// folded into the same empty result, since silently returning an empty list
/// here would reproduce the exact ancestor-loss bug this module exists to
/// prevent, just via a different trigger than the one save is guarding
/// against.
fn load_ancestor_session_ids(sessions_dir: &Path, session_id: &str) -> Vec<String> {
    load_session_field(
        sessions_dir,
        session_id,
        "agent.resurrect.save.ancestor_load_failed",
        |s| s.ancestor_session_ids,
    )
}

/// Shared "read one field off the stored session, defaulting on a missing
/// file, or on a read error (logged as `log_event`)" pattern behind
/// `load_engine` and `load_ancestor_session_ids`.
fn load_session_field<T: Default>(
    sessions_dir: &Path,
    session_id: &str,
    log_event: &'static str,
    field: impl FnOnce(Session) -> T,
) -> T {
    match store::load_session_from(sessions_dir, session_id) {
        Ok(session) => session.map(field).unwrap_or_default(),
        Err(error) => {
            tracing::warn!(event = log_event, session_id = %session_id, %error);
            T::default()
        }
    }
}

/// Restores pane session IDs from the state file.
///
/// Reads the state file and, for each pane that still exists, sets the user option
/// and types `a agent resume <session-id>` into the pane to re-launch Claude Code,
/// unless the pane already has a live agent process (see `resume_command_for`).
/// The session ID is passed as an argument (instead of relying on the pane option)
/// so the resumed process is not racing against tmux to observe the just-set option.
/// Each session's stored pane_id is also corrected here — see
/// `store::update_session_tmux_pane_id_in` for why.
///
/// tmux-resurrect's own `@resurrect-processes` is not used: its per-pane full-command
/// field may contain multiple lines (one per shell child) which confuses the awk-based
/// parser in `restore_all_pane_processes`, and process restoration runs before our
/// post-restore hook, so the option would not yet be set anyway.
fn run_restore(_args: &RestoreArgs) -> Result<()> {
    let run_id = short_run_id();
    let span = tracing::info_span!("agent.resurrect.restore", run_id = %run_id);
    let _entered = span.enter();

    let state_file = state_file_path()?;
    let sessions_dir = store::sessions_dir()?;

    if !state_file.exists() {
        // No state file means nothing to restore
        tracing::info!(
            event = "agent.resurrect.restore.skip",
            reason = "no state file"
        );
        return Ok(());
    }

    let pane_sessions = read_state_file(&state_file)?;

    tracing::info!(
        event = "agent.resurrect.restore.start",
        pane_count = pane_sessions.len(),
    );

    if pane_sessions.is_empty() {
        tracing::info!(
            event = "agent.resurrect.restore.skip",
            reason = "empty state file"
        );
        return Ok(());
    }

    // Captured once and shared across every pane below: `ProcessSnapshot::capture`
    // forks `ps -A` to read the whole system process table, so re-capturing it
    // per pane would fork once per restored pane instead of once per restore.
    let snapshot = ProcessSnapshot::capture();

    let panes_by_position: HashMap<String, (String, u32)> = tmux::list_all_panes()
        .unwrap_or_default()
        .into_iter()
        .map(|pane| {
            (
                format_pane_position(&pane.session_name, pane.window_index, pane.pane_index),
                (pane.pane_id, pane.pane_pid),
            )
        })
        .collect();

    let mut commands = Vec::new();
    let mut queued_count = 0;
    for (pane_position, (session_id, ancestor_session_ids, engine)) in &pane_sessions {
        let Some((pane_id, pane_pid)) = panes_by_position.get(pane_position) else {
            tracing::warn!(event = "agent.resurrect.restore.pane_skipped", pane_position = %pane_position);
            continue;
        };

        // Must run before `commands` is sent below: only then can no
        // sibling session's SessionStart hook observe a still-stale pane_id
        // (see `store::update_session_tmux_pane_id_in`).
        if let Err(error) =
            store::update_session_tmux_pane_id_in(&sessions_dir, session_id, pane_id)
        {
            tracing::warn!(
                event = "agent.resurrect.restore.pane_id_update_failed",
                session_id = %session_id,
                %error,
            );
        }

        let engine = engine.unwrap_or_else(|| load_engine(&sessions_dir, session_id));
        let pane_has_agent =
            pane::process::pane_has_live_agent_process(*pane_pid, engine, snapshot.as_ref());
        commands.extend(restore_commands_for_pane(
            pane_id,
            session_id,
            ancestor_session_ids,
            engine,
            pane_has_agent,
        ));

        queued_count += 1;
        tracing::info!(event = "agent.resurrect.restore.pane_queued", pane_position = %pane_position);
    }

    let batch_result = tmux::run_batch(&commands);
    if let Err(error) = &batch_result {
        tracing::warn!(event = "agent.resurrect.restore.batch_failed", %error);
    }

    tracing::info!(
        event = "agent.resurrect.restore.summary",
        queued = queued_count,
        total = pane_sessions.len(),
        batch_ok = batch_result.is_ok(),
    );

    // Only clean up the state file once the batch actually ran: if it failed
    // (e.g. the tmux server was briefly unavailable), keeping the file lets
    // the next restore attempt retry instead of losing the pane mapping.
    if queued_count > 0 && batch_result.is_ok() {
        let _ = fs::remove_file(&state_file);
    }

    Ok(())
}

/// Builds the tmux commands to restore one pane: a `set-option` for the
/// session id, plus an optional `send-keys` from `resume_command_for`.
fn restore_commands_for_pane(
    pane_id: &str,
    session_id: &str,
    ancestor_session_ids: &[String],
    engine: Engine,
    pane_has_agent: bool,
) -> Vec<Vec<String>> {
    let mut commands = vec![vec![
        "set-option".to_string(),
        "-p".to_string(),
        "-t".to_string(),
        pane_id.to_string(),
        TMUX_SESSION_OPTION.to_string(),
        session_id.to_string(),
    ]];

    if let Some(command) =
        resume_command_for(session_id, ancestor_session_ids, engine, pane_has_agent)
    {
        commands.push(vec![
            "send-keys".to_string(),
            "-t".to_string(),
            pane_id.to_string(),
            command,
            "Enter".to_string(),
        ]);
    }

    commands
}

/// Builds the `a agent resume <session_id>` command to type into a pane, or
/// `None` when the pane must not be touched.
///
/// `pane_has_agent` reflects whether the pane's process tree already has a
/// live process for the session's engine (see
/// `pane::process::pane_has_live_agent_process`).
/// A pane carries `TMUX_SESSION_OPTION` for as long as a session ever ran
/// there -- including one that is still active with a live process reading
/// the pane's input -- so restoring the option is always safe, but typing
/// text into that pane is not: it would land in the middle of whatever the
/// user or Claude Code is doing.
///
/// `ancestor_session_ids`, when non-empty, is passed via
/// `--ancestor-session-ids` so `a agent resume` can set
/// `ARMYKNIFE_ANCESTOR_SESSION_IDS` on the `claude` process it execs.
/// `engine` is passed explicitly because the store record may have been
/// removed before restore runs.
fn resume_command_for(
    session_id: &str,
    ancestor_session_ids: &[String],
    engine: Engine,
    pane_has_agent: bool,
) -> Option<String> {
    if pane_has_agent {
        return None;
    }
    // Quoted because `send-keys` types into an interactive shell where
    // metacharacters (spaces, `;`, backticks) would otherwise be parsed.
    let quoted_id = shlex::try_quote(session_id)
        .map(|cow| cow.into_owned())
        .unwrap_or_else(|_| session_id.to_string());
    let mut command = format!(
        "a agent resume {quoted_id} --engine {}",
        format_engine(engine)
    );
    if !ancestor_session_ids.is_empty() {
        let joined = ancestor_session_ids.join(",");
        let quoted_ancestors = shlex::try_quote(&joined)
            .map(|cow| cow.into_owned())
            .unwrap_or_else(|_| joined);
        command.push_str(&format!(" --ancestor-session-ids {quoted_ancestors}"));
    }
    Some(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use rstest::rstest;
    use tempfile::TempDir;

    #[test]
    fn state_file_path_is_under_cache_dir() {
        let path = state_file_path().expect("should return path");
        assert!(path.ends_with("cc/resurrect/pane_sessions.txt"));
    }

    #[rstest]
    #[case::valid_line_no_ancestors(
        "main:0.1\tabc-123",
        Some(("main:0.1", "abc-123", Vec::new(), None))
    )]
    #[case::valid_uuid(
        "work:2.0\t550e8400-e29b-41d4-a716-446655440000",
        Some((
            "work:2.0",
            "550e8400-e29b-41d4-a716-446655440000",
            Vec::new(),
            None
        ))
    )]
    #[case::session_with_slash(
        "fohte/repo:1.2\txyz-456",
        Some(("fohte/repo:1.2", "xyz-456", Vec::new(), None))
    )]
    #[case::with_ancestors(
        "main:0.1\tabc-123\troot-1,parent-1",
        Some((
            "main:0.1",
            "abc-123",
            vec!["root-1".to_string(), "parent-1".to_string()],
            None
        ))
    )]
    #[case::with_empty_ancestors_field(
        "main:0.1\tabc-123\t",
        Some(("main:0.1", "abc-123", Vec::new(), None))
    )]
    #[case::with_codex_engine(
        "main:0.1\tabc-123\troot-1\tcodex",
        Some((
            "main:0.1",
            "abc-123",
            vec!["root-1".to_string()],
            Some(Engine::Codex)
        ))
    )]
    #[case::missing_tab("main:0.1abc-123", None)]
    #[case::unknown_engine("main:0.1\tabc-123\t\tunknown", None)]
    #[case::too_many_tabs("main:0.1\tabc\t123\textra", None)]
    #[case::empty_line("", None)]
    fn test_parse_state_line(
        #[case] line: &str,
        #[case] expected: Option<(&str, &str, Vec<String>, Option<Engine>)>,
    ) {
        assert_eq!(parse_state_line(line), expected);
    }

    #[rstest]
    #[case::simple("main", 0, 1, "main:0.1")]
    #[case::high_indices("work", 10, 5, "work:10.5")]
    #[case::session_with_slash("fohte/repo", 1, 2, "fohte/repo:1.2")]
    fn test_format_pane_position(
        #[case] session_name: &str,
        #[case] window_index: u32,
        #[case] pane_index: u32,
        #[case] expected: &str,
    ) {
        assert_eq!(
            format_pane_position(session_name, window_index, pane_index),
            expected
        );
    }

    #[test]
    fn write_and_read_state_file_roundtrip() {
        let temp_dir = TempDir::new().expect("should create temp dir");
        let state_file = temp_dir.path().join("pane_sessions.txt");

        let pane_sessions = vec![
            (
                "main".to_string(),
                0,
                1,
                "abc-123".to_string(),
                Vec::new(),
                Engine::Claude,
            ),
            (
                "work".to_string(),
                2,
                0,
                "def-456".to_string(),
                vec!["root-1".to_string(), "parent-1".to_string()],
                Engine::Codex,
            ),
            (
                "fohte/repo".to_string(),
                1,
                2,
                "550e8400-e29b-41d4-a716-446655440000".to_string(),
                Vec::new(),
                Engine::Claude,
            ),
        ];

        write_state_file(&state_file, &pane_sessions).expect("should write state file");

        let read_sessions = read_state_file(&state_file).expect("should read state file");

        assert_eq!(
            read_sessions,
            HashMap::from([
                (
                    "main:0.1".to_string(),
                    ("abc-123".to_string(), Vec::new(), Some(Engine::Claude))
                ),
                (
                    "work:2.0".to_string(),
                    (
                        "def-456".to_string(),
                        vec!["root-1".to_string(), "parent-1".to_string()],
                        Some(Engine::Codex)
                    )
                ),
                (
                    "fohte/repo:1.2".to_string(),
                    (
                        "550e8400-e29b-41d4-a716-446655440000".to_string(),
                        Vec::new(),
                        Some(Engine::Claude)
                    )
                ),
            ])
        );
    }

    #[test]
    fn write_state_file_creates_parent_directories() {
        let temp_dir = TempDir::new().expect("should create temp dir");
        let state_file = temp_dir.path().join("nested").join("dir").join("state.txt");

        let pane_sessions = vec![(
            "main".to_string(),
            0,
            0,
            "session-id".to_string(),
            Vec::new(),
            Engine::Claude,
        )];

        write_state_file(&state_file, &pane_sessions).expect("should write state file");
        assert_eq!(
            read_state_file(&state_file).expect("should read state file"),
            HashMap::from([(
                "main:0.0".to_string(),
                ("session-id".to_string(), Vec::new(), Some(Engine::Claude),),
            )])
        );
    }

    #[test]
    fn read_state_file_skips_malformed_lines() {
        let temp_dir = TempDir::new().expect("should create temp dir");
        let state_file = temp_dir.path().join("state.txt");

        // Write a file with some malformed lines
        fs::write(
            &state_file,
            indoc! {"
                main:0.1\tabc-123
                malformed line

                work:2.0\tdef-456\troot-1
                work-codex:3.0\tghi-789\troot-2\tcodex
                extra\ttabs\there\ttoo\tmany
            "},
        )
        .expect("should write file");

        let sessions = read_state_file(&state_file).expect("should read state file");

        assert_eq!(
            sessions,
            HashMap::from([
                (
                    "main:0.1".to_string(),
                    ("abc-123".to_string(), Vec::new(), None)
                ),
                (
                    "work:2.0".to_string(),
                    ("def-456".to_string(), vec!["root-1".to_string()], None)
                ),
                (
                    "work-codex:3.0".to_string(),
                    (
                        "ghi-789".to_string(),
                        vec!["root-2".to_string()],
                        Some(Engine::Codex)
                    )
                ),
            ])
        );
    }

    #[test]
    fn read_state_file_returns_empty_map_for_empty_file() {
        let temp_dir = TempDir::new().expect("should create temp dir");
        let state_file = temp_dir.path().join("state.txt");

        fs::write(&state_file, "").expect("should write file");

        let sessions = read_state_file(&state_file).expect("should read state file");

        assert_eq!(sessions, HashMap::new());
    }

    mod load_ancestor_session_ids_tests {
        use std::collections::BTreeSet;

        use chrono::Utc;

        use super::*;
        use crate::commands::agent::types::{Engine, Session, SessionStatus};

        fn session_with_ancestors(id: &str, ancestor_session_ids: Vec<String>) -> Session {
            Session {
                session_id: id.to_string(),
                cwd: PathBuf::from("/tmp/test"),
                transcript_path: None,
                tty: None,
                tmux_info: None,
                status: SessionStatus::Running,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                last_message: None,
                current_tool: None,
                label: None,
                ancestor_session_ids,
                pending_bg_task_ids: BTreeSet::new(),
                pending_agent_task_ids: BTreeSet::new(),
                pending_permission_agent_ids: BTreeSet::new(),
                pending_permission_request_ids: Default::default(),
                read_at: None,
                sweep_signaled: false,
                engine: Engine::Claude,
            }
        }

        #[test]
        fn returns_ancestors_recorded_on_the_session() {
            let sessions_dir = TempDir::new().expect("temp dir");
            let session = session_with_ancestors(
                "sess-1",
                vec!["root-1".to_string(), "parent-1".to_string()],
            );
            store::save_session_to(sessions_dir.path(), &session).expect("save session");

            assert_eq!(
                load_ancestor_session_ids(sessions_dir.path(), "sess-1"),
                vec!["root-1".to_string(), "parent-1".to_string()]
            );
        }

        #[test]
        fn returns_empty_for_missing_session() {
            let sessions_dir = TempDir::new().expect("temp dir");

            assert_eq!(
                load_ancestor_session_ids(sessions_dir.path(), "no-such-session"),
                Vec::<String>::new()
            );
        }
    }

    mod load_engine_tests {
        use std::collections::BTreeSet;

        use chrono::Utc;

        use super::*;
        use crate::commands::agent::types::{Engine, Session, SessionStatus};

        fn session_with_engine(id: &str, engine: Engine) -> Session {
            Session {
                session_id: id.to_string(),
                cwd: PathBuf::from("/tmp/test"),
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
                engine,
            }
        }

        #[test]
        fn returns_the_engine_recorded_on_the_session() {
            let sessions_dir = TempDir::new().expect("temp dir");
            let session = session_with_engine("sess-1", Engine::Codex);
            store::save_session_to(sessions_dir.path(), &session).expect("save session");

            assert_eq!(load_engine(sessions_dir.path(), "sess-1"), Engine::Codex);
        }

        #[test]
        fn defaults_to_claude_for_missing_session() {
            let sessions_dir = TempDir::new().expect("temp dir");

            assert_eq!(
                load_engine(sessions_dir.path(), "no-such-session"),
                Engine::Claude
            );
        }
    }

    mod resume_command_for_tests {
        use super::*;

        // A pane whose process tree already has a live claude process must
        // never be typed into; the resume command would land mid-conversation.
        #[rstest]
        #[case::pane_has_claude("abc-123", &[], Engine::Claude, true, None)]
        #[case::pane_is_free_no_ancestors(
            "abc-123",
            &[],
            Engine::Claude,
            false,
            Some("a agent resume abc-123 --engine claude".to_string())
        )]
        #[case::pane_is_free_with_ancestors(
            "abc-123",
            &["root-1".to_string(), "parent-1".to_string()],
            Engine::Claude,
            false,
            Some("a agent resume abc-123 --engine claude --ancestor-session-ids 'root-1,parent-1'".to_string())
        )]
        #[case::pane_is_free_codex(
            "abc-123",
            &[],
            Engine::Codex,
            false,
            Some("a agent resume abc-123 --engine codex".to_string())
        )]
        #[case::quotes_metacharacters(
            "id; rm -rf /",
            &[],
            Engine::Claude,
            false,
            Some("a agent resume 'id; rm -rf /' --engine claude".to_string())
        )]
        fn resume_command_for_cases(
            #[case] session_id: &str,
            #[case] ancestor_session_ids: &[String],
            #[case] engine: Engine,
            #[case] pane_has_claude: bool,
            #[case] expected: Option<String>,
        ) {
            assert_eq!(
                resume_command_for(session_id, ancestor_session_ids, engine, pane_has_claude),
                expected
            );
        }
    }

    mod restore_commands_for_pane_tests {
        use super::*;

        #[rstest]
        #[case::pane_has_claude(
            "%5",
            "abc-123",
            &[],
            Engine::Claude,
            true,
            vec![vec![
                "set-option".to_string(),
                "-p".to_string(),
                "-t".to_string(),
                "%5".to_string(),
                TMUX_SESSION_OPTION.to_string(),
                "abc-123".to_string(),
            ]]
        )]
        #[case::pane_is_free_no_ancestors(
            "%5",
            "abc-123",
            &[],
            Engine::Claude,
            false,
            vec![
                vec![
                    "set-option".to_string(),
                    "-p".to_string(),
                    "-t".to_string(),
                    "%5".to_string(),
                    TMUX_SESSION_OPTION.to_string(),
                    "abc-123".to_string(),
                ],
                vec![
                    "send-keys".to_string(),
                    "-t".to_string(),
                    "%5".to_string(),
                    "a agent resume abc-123 --engine claude".to_string(),
                    "Enter".to_string(),
                ],
            ]
        )]
        #[case::pane_is_free_with_ancestors(
            "%5",
            "abc-123",
            &["root-1".to_string(), "parent-1".to_string()],
            Engine::Codex,
            false,
            vec![
                vec![
                    "set-option".to_string(),
                    "-p".to_string(),
                    "-t".to_string(),
                    "%5".to_string(),
                    TMUX_SESSION_OPTION.to_string(),
                    "abc-123".to_string(),
                ],
                vec![
                    "send-keys".to_string(),
                    "-t".to_string(),
                    "%5".to_string(),
                    "a agent resume abc-123 --engine codex --ancestor-session-ids 'root-1,parent-1'".to_string(),
                    "Enter".to_string(),
                ],
            ]
        )]
        fn restore_commands_for_pane_cases(
            #[case] pane_id: &str,
            #[case] session_id: &str,
            #[case] ancestor_session_ids: &[String],
            #[case] engine: Engine,
            #[case] pane_has_claude: bool,
            #[case] expected: Vec<Vec<String>>,
        ) {
            assert_eq!(
                restore_commands_for_pane(
                    pane_id,
                    session_id,
                    ancestor_session_ids,
                    engine,
                    pane_has_claude
                ),
                expected
            );
        }
    }
}
