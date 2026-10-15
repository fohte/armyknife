//! Tmux-resurrect integration for Claude Code session restoration.
//!
//! This module provides commands to save and restore Claude Code session IDs
//! when tmux-resurrect saves/restores tmux sessions. Since tmux user options
//! are not automatically preserved by tmux-resurrect, we need to explicitly
//! save them to a state file and restore them after tmux-resurrect completes.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::Result;
use clap::{Args, Subcommand};

use super::pane;
use super::store;
use super::types::{Engine, Session, TMUX_SESSION_OPTION, TMUX_SESSION_OPTION_LEGACY};
use crate::infra::process::ProcessSnapshot;
use crate::infra::tmux;
use crate::shared::log::short_run_id;

mod state_file;
use state_file::{
    format_engine, format_pane_position, read_state_file, state_file_path, write_state_file,
};

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
    use rstest::rstest;
    use std::path::PathBuf;
    use tempfile::TempDir;

    mod load_ancestor_session_ids_tests {
        use std::collections::BTreeSet;

        use chrono::Utc;

        use super::*;
        use crate::commands::agent::types::{Engine, Session, SessionStatus};

        fn session_with_ancestors(id: &str, ancestor_session_ids: Vec<String>) -> Session {
            Session {
                pr_context: None,
                session_id: id.to_string(),
                work_type: None,
                work_type_pinned: false,
                vibe_enabled: false,
                is_delegated: false,
                crit_urls: Vec::new(),
                pending_human_review_ids: Default::default(),
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
                stop_blocked_by_armyknife: false,
                agent_status: None,
                agent_status_clear_pending: false,
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
                pr_context: None,
                session_id: id.to_string(),
                work_type: None,
                work_type_pinned: false,
                vibe_enabled: false,
                is_delegated: false,
                crit_urls: Vec::new(),
                pending_human_review_ids: Default::default(),
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
                stop_blocked_by_armyknife: false,
                agent_status: None,
                agent_status_clear_pending: false,
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
