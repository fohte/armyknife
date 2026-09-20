use std::path::Path;

use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use clap::Args;
use thiserror::Error;

use super::store;
use super::types::{
    Engine, Session, SessionStatus, TMUX_SESSION_OPTION, TmuxInfo, resolve_session_option,
};
use crate::infra::{process, tmux};
use crate::shared::command::{self, find_command_path};
use crate::shared::env_var::EnvVars;

mod session_metadata;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ResumeArgs {
    /// Agent session ID to resume. When omitted, the session ID is read from the
    /// current tmux pane's `@armyknife-last-agent-session-id` user option.
    pub session_id: Option<String>,

    /// Comma-separated ancestor session IDs (root to immediate parent) to restore.
    /// `a agent resurrect restore` passes this because a respawned pane otherwise
    /// carries no ancestor information. Codex records it before resuming because
    /// a shared daemon cannot inherit this process's environment; Claude and an
    /// embedded Codex process also receive it via `ARMYKNIFE_ANCESTOR_SESSION_IDS`.
    #[arg(long)]
    pub ancestor_session_ids: Option<String>,
}

/// Runs the resume command.
/// If a session ID argument is provided, resumes that session directly.
/// Otherwise, reads the session ID from the current tmux pane's user option.
pub fn run(args: &ResumeArgs) -> Result<()> {
    let session_id = match args.session_id.as_deref() {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => resolve_session_id_from_pane()?,
    };

    // A missing store record (e.g. a tmux-resurrect restore racing
    // `cleanup_stale_sessions`, see `resurrect.rs`) must not block resuming
    // -- fall back to the default engine and let `claude --resume` itself
    // report an unknown session ID.
    let engine = store::load_session(&session_id)?
        .map(|s| s.engine)
        .unwrap_or_default();

    let (binary_name, resume_args) = resume_binary_and_args(engine, &session_id);

    let binary_path = find_command_path(binary_name)
        .ok_or_else(|| anyhow::anyhow!("Could not find '{binary_name}' command in PATH"))?;

    let ancestor_session_ids = args
        .ancestor_session_ids
        .as_deref()
        .filter(|s| !s.is_empty());

    // Codex delays its resume SessionStart hook until the first turn. Make
    // the restored session visible while it is waiting for that turn.
    if engine == Engine::Codex {
        if let Some(ancestor_session_ids) = ancestor_session_ids {
            session_metadata::record_ancestor_session_ids_if_empty(
                &store::sessions_dir()?,
                &session_id,
                ancestor_session_ids,
            )?;
        }
        return run_codex_resume_with_status(&session_id, || {
            run_codex_resume(&binary_path, resume_args, ancestor_session_ids)
        });
    }

    let err = match ancestor_session_ids {
        Some(ancestor_ids) => process::exec_replace_with_env(
            &binary_path,
            resume_args,
            &[(EnvVars::ancestor_session_ids_name(), ancestor_ids)],
        ),
        None => process::exec_replace(&binary_path, resume_args),
    };

    bail!("Failed to exec {}: {}", binary_name, err)
}

/// Runs Codex as a child so an unsuccessful resume can restore the ended
/// status. The child keeps the invoking terminal attached for the interactive
/// TUI, while Claude keeps the existing `exec_replace` path in [`run`].
fn run_codex_resume(
    binary_path: &Path,
    resume_args: Vec<String>,
    ancestor_session_ids: Option<&str>,
) -> Result<()> {
    let mut command = command::new(binary_path);
    command.args(resume_args);
    if let Some(ancestor_session_ids) = ancestor_session_ids {
        command.env(EnvVars::ancestor_session_ids_name(), ancestor_session_ids);
    }
    let status = command
        .status()
        .map_err(|error| anyhow::anyhow!("Failed to start codex: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        bail!("codex resume exited with status {status}")
    }
}

/// Captures the fields needed to undo the resume transition if Codex exits
/// unsuccessfully.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ResumeStatusChange {
    original_read_at: Option<DateTime<Utc>>,
    original_updated_at: DateTime<Utc>,
    original_tmux_info: Option<TmuxInfo>,
    marked_at: DateTime<Utc>,
}

/// Runs a Codex resume while making its delayed first-turn hook transition
/// visible in the session store.
fn run_codex_resume_with_status(
    session_id: &str,
    launch: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let sessions_dir = store::sessions_dir()?;
    run_codex_resume_with_status_in(&sessions_dir, session_id, current_tmux_info(), launch)
}

fn run_codex_resume_with_status_in(
    sessions_dir: &Path,
    session_id: &str,
    current_tmux_info: Option<TmuxInfo>,
    launch: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let status_change = mark_codex_session_resumed_in(sessions_dir, session_id, current_tmux_info)?;
    let result = launch();
    if let Err(error) = result {
        if let Some(change) = status_change
            && let Err(restore_error) =
                restore_failed_codex_resume_in(sessions_dir, session_id, &change)
        {
            eprintln!(
                "[armyknife] warning: failed to restore session status after Codex resume failure: {restore_error}"
            );
        }
        return Err(error);
    }
    Ok(())
}

/// Finds the pane that invoked `a agent resume`, if the command runs in tmux.
/// The stored pane must follow the resumed process so stale-pane cleanup does
/// not remove a session merely because its old pane was replaced.
fn current_tmux_info() -> Option<TmuxInfo> {
    tmux::get_pane_info_by_pid(std::process::id()).map(|pane| TmuxInfo {
        session_name: pane.session_name,
        window_name: pane.window_name,
        window_index: pane.window_index,
        pane_id: pane.pane_id,
    })
}

/// Marks an ended Codex session as stopped before launching `codex resume`.
/// The lock keeps this transition from racing with a hook that updates the
/// same session file.
fn mark_codex_session_resumed_in(
    sessions_dir: &Path,
    session_id: &str,
    current_tmux_info: Option<TmuxInfo>,
) -> Result<Option<ResumeStatusChange>> {
    let mut change = None;
    store::update_session_in(sessions_dir, session_id, |session| {
        if session.engine != Engine::Codex || session.status != SessionStatus::Ended {
            return false;
        }

        let marked_at = Utc::now();
        change = Some(ResumeStatusChange {
            original_read_at: session.read_at,
            original_updated_at: session.updated_at,
            original_tmux_info: session.tmux_info.clone(),
            marked_at,
        });
        session.status = SessionStatus::Stopped;
        session.read_at = None;
        session.updated_at = marked_at;
        session.tmux_info = current_tmux_info;
        true
    })?;
    Ok(change)
}

/// Restores the pre-resume fields only when no hook has updated the session
/// since the transition. Other fields are kept so a concurrent metadata update
/// cannot be overwritten by a failed resume rollback.
fn restore_failed_codex_resume_in(
    sessions_dir: &Path,
    session_id: &str,
    change: &ResumeStatusChange,
) -> Result<()> {
    store::update_session_in(sessions_dir, session_id, |session| {
        if session.status != SessionStatus::Stopped || session.updated_at != change.marked_at {
            return false;
        }

        session.status = SessionStatus::Ended;
        session.read_at = change.original_read_at;
        session.updated_at = change.original_updated_at;
        session.tmux_info = change.original_tmux_info.clone();
        true
    })
}

/// Binary name and CLI args needed to resume `session_id` for `engine`.
/// Codex's resume is a subcommand (`codex resume <id>`), not a flag like
/// Claude Code's `--resume`.
fn resume_binary_and_args(engine: Engine, session_id: &str) -> (&'static str, Vec<String>) {
    match engine {
        Engine::Claude => (
            "claude",
            vec!["--resume".to_string(), session_id.to_string()],
        ),
        Engine::Codex => ("codex", vec!["resume".to_string(), session_id.to_string()]),
    }
}

/// Also used by `peer::me` to resolve the session running in the caller's
/// own pane, mirroring what `resume` does to find the session to relaunch.
pub(crate) fn resolve_session_id_from_pane() -> Result<String> {
    let pane_id = current_pane_id()?;
    let pane_option = resolve_session_option(|option| tmux::get_pane_option(&pane_id, option));
    session_id_from_pane_option(&pane_id, pane_option.as_deref())
}

/// Pure decision half of [`resolve_session_id_from_pane`], split out so the
/// "pane option not set or empty" failure is testable without a live tmux
/// server (unlike `tmux::get_pane_option`, which shells out).
fn session_id_from_pane_option(pane_id: &str, pane_option: Option<&str>) -> Result<String> {
    pane_option
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No agent session ID found for pane {} (option '{}' not set or empty)",
                pane_id,
                TMUX_SESSION_OPTION
            )
        })
}

/// Returns the tmux pane ID of the caller, read from `$TMUX_PANE`.
///
/// Resolving by `$TMUX_PANE` (set by tmux when it spawns the pane's process)
/// rather than by tmux's notion of the focused pane is required so that resume
/// targets the pane that invoked the command even if the user switches focus
/// before tmux can answer.
fn current_pane_id() -> Result<String> {
    tmux::current_pane_id_from_env()
        .ok_or_else(|| anyhow::anyhow!("Not running inside a tmux pane: $TMUX_PANE is not set"))
}

/// Programs a `Paused` session's pane may be sitting at for its respawn to
/// be safe. Anything else means a foreground program is running that a
/// blind `respawn-pane -k` would kill.
const SHELL_COMMANDS: &[&str] = &["zsh", "bash", "fish", "sh", "dash"];

/// Failure modes of [`respawn_paused_session`]. Both the TUI's resume key
/// and `a agent peer wake` render these as user-facing messages.
#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum RespawnError {
    #[error("No tmux pane for this session")]
    NoTmuxPane,
    #[error("Session is not paused")]
    NotPaused,
    #[error("Pane is running `{0}`, cannot resume")]
    PaneBusy(String),
    #[error("Cannot read pane state")]
    PaneStateUnreadable,
    #[error("Failed to build resume command")]
    CommandBuildFailed,
    #[error("Failed to respawn pane: {0}")]
    RespawnFailed(String),
}

/// Replaces a paused session's pane's root process with `a agent resume`
/// wrapped in the user's login shell, so the selected agent CLI restarts in it.
/// Does not focus the pane -- callers that want that (the TUI) do it
/// themselves afterward, since a resume triggered from another session must
/// not steal the user's tmux focus.
///
/// The session actually resumed is whichever one is recorded on the pane's
/// `TMUX_SESSION_OPTION`, not necessarily `session.session_id` -- callers
/// that can't guarantee the two are in sync must verify this themselves
/// (see `peer::wake::check_pane_matches_target`).
///
/// Returns the pane ID that was respawned on success.
pub(crate) fn respawn_paused_session(session: &Session) -> Result<String, RespawnError> {
    let tmux_info = session.tmux_info.as_ref().ok_or(RespawnError::NoTmuxPane)?;
    if session.status != SessionStatus::Paused {
        return Err(RespawnError::NotPaused);
    }
    check_idle_at_shell_prompt(tmux::get_pane_current_command(&tmux_info.pane_id).as_deref())?;

    let wrapped = build_resume_command().ok_or(RespawnError::CommandBuildFailed)?;
    tmux::respawn_pane(&tmux_info.pane_id, &wrapped)
        .map_err(|e| RespawnError::RespawnFailed(e.to_string()))?;

    Ok(tmux_info.pane_id.clone())
}

/// Only respawn if the pane is sitting at a shell prompt. If the user
/// started another program in the pane, a blind respawn must not kill it
/// silently.
fn check_idle_at_shell_prompt(pane_current_command: Option<&str>) -> Result<(), RespawnError> {
    match pane_current_command {
        Some(cmd) if SHELL_COMMANDS.contains(&cmd) => Ok(()),
        Some(cmd) => Err(RespawnError::PaneBusy(cmd.to_string())),
        None => Err(RespawnError::PaneStateUnreadable),
    }
}

/// Wraps `a agent resume` in the user's login shell so that when claude exits
/// normally, control returns to a shell prompt instead of tmux closing the
/// pane (`respawn-pane` replaces the pane's root process).
///
/// `-i` is required on the outer shell: `a agent resume` looks up the selected
/// agent CLI in $PATH via `find_command_path`, and many users only extend
/// $PATH in their interactive rc file (e.g. `.zshrc`). Running without `-i`
/// would inherit tmux's pre-rc $PATH and fail to locate the CLI.
fn build_resume_command() -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_else(|| "a".to_string());
    let inner = shlex::try_join([exe.as_str(), "agent", "resume"]).ok()?;
    let exec_shell = shlex::try_join([shell.as_str(), "-i"]).ok()?;
    let script = format!("{inner}; exec {exec_shell}");
    shlex::try_join([shell.as_str(), "-i", "-c", &script]).ok()
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::returns_value_when_set(Some("%12"), Ok("%12".to_string()))]
    #[case::errors_when_unset(
        None,
        Err("Not running inside a tmux pane: $TMUX_PANE is not set".to_string())
    )]
    #[case::errors_when_empty(
        Some(""),
        Err("Not running inside a tmux pane: $TMUX_PANE is not set".to_string())
    )]
    fn current_pane_id_cases(
        #[case] env_value: Option<&str>,
        #[case] expected: std::result::Result<String, String>,
    ) {
        temp_env::with_vars([("TMUX_PANE", env_value)], || {
            assert_eq!(current_pane_id().map_err(|e| e.to_string()), expected);
        });
    }

    mod session_id_from_pane_option_tests {
        use super::*;

        #[rstest]
        #[case::returns_the_value_when_set(Some("abc123"), Ok("abc123".to_string()))]
        #[case::errors_when_unset(
            None,
            Err(
                "No agent session ID found for pane %5 (option '@armyknife-last-agent-session-id' not set or empty)"
                    .to_string()
            )
        )]
        #[case::errors_when_empty(
            Some(""),
            Err(
                "No agent session ID found for pane %5 (option '@armyknife-last-agent-session-id' not set or empty)"
                    .to_string()
            )
        )]
        fn cases(
            #[case] pane_option: Option<&str>,
            #[case] expected: std::result::Result<String, String>,
        ) {
            assert_eq!(
                session_id_from_pane_option("%5", pane_option).map_err(|e| e.to_string()),
                expected
            );
        }
    }

    mod check_idle_at_shell_prompt_tests {
        use super::*;

        #[rstest]
        #[case::zsh(Some("zsh"), Ok(()))]
        #[case::bash(Some("bash"), Ok(()))]
        #[case::other_program(Some("nvim"), Err(RespawnError::PaneBusy("nvim".to_string())))]
        #[case::unreadable(None, Err(RespawnError::PaneStateUnreadable))]
        fn cases(
            #[case] pane_current_command: Option<&str>,
            #[case] expected: std::result::Result<(), RespawnError>,
        ) {
            assert_eq!(check_idle_at_shell_prompt(pane_current_command), expected);
        }
    }

    mod resume_binary_and_args_tests {
        use super::*;

        #[rstest]
        #[case::claude(
            Engine::Claude,
            "session-1",
            ("claude", vec!["--resume".to_string(), "session-1".to_string()])
        )]
        #[case::codex(
            Engine::Codex,
            "session-1",
            ("codex", vec!["resume".to_string(), "session-1".to_string()])
        )]
        fn cases(
            #[case] engine: Engine,
            #[case] session_id: &str,
            #[case] expected: (&str, Vec<String>),
        ) {
            assert_eq!(resume_binary_and_args(engine, session_id), expected);
        }
    }

    mod codex_resume_status_tests {
        use std::path::PathBuf;

        use rstest::{fixture, rstest};
        use tempfile::TempDir;

        use super::*;

        #[fixture]
        fn temp_dir() -> TempDir {
            TempDir::new().expect("temp dir creation should succeed")
        }

        fn session(engine: Engine, status: SessionStatus) -> Session {
            let now = Utc::now();
            Session {
                session_id: "resume-target".to_string(),
                cwd: PathBuf::from("/tmp/test"),
                transcript_path: None,
                tty: None,
                tmux_info: None,
                status,
                created_at: now,
                updated_at: now,
                last_message: None,
                current_tool: None,
                label: None,
                ancestor_session_ids: Vec::new(),
                pending_bg_task_ids: Default::default(),
                pending_agent_task_ids: Default::default(),
                pending_permission_agent_ids: Default::default(),
                read_at: Some(now),
                sweep_signaled: false,
                engine,
            }
        }

        #[rstest]
        #[case::codex_ended(Engine::Codex, SessionStatus::Ended, true, SessionStatus::Stopped)]
        #[case::codex_stopped(Engine::Codex, SessionStatus::Stopped, false, SessionStatus::Stopped)]
        #[case::codex_paused(Engine::Codex, SessionStatus::Paused, false, SessionStatus::Paused)]
        #[case::claude_ended(Engine::Claude, SessionStatus::Ended, false, SessionStatus::Ended)]
        fn marks_only_ended_codex_sessions(
            temp_dir: TempDir,
            #[case] engine: Engine,
            #[case] status: SessionStatus,
            #[case] expected_change: bool,
            #[case] expected_status: SessionStatus,
        ) {
            let original = session(engine, status);
            let expected_read_at = original.read_at;
            store::save_session_to(temp_dir.path(), &original).expect("save should succeed");

            let change = mark_codex_session_resumed_in(temp_dir.path(), "resume-target", None)
                .expect("mark should succeed");
            let reloaded = store::load_session_from(temp_dir.path(), "resume-target")
                .expect("load should succeed")
                .expect("session should exist");

            assert_eq!(
                (change.is_some(), reloaded.status, reloaded.read_at),
                (
                    expected_change,
                    expected_status,
                    if expected_change {
                        None
                    } else {
                        expected_read_at
                    },
                )
            );
        }

        #[rstest]
        fn failed_resume_restores_codex_session(temp_dir: TempDir) {
            let original = session(Engine::Codex, SessionStatus::Ended);
            let expected = (
                original.status,
                original.read_at,
                original.updated_at,
                original.tmux_info.clone(),
            );
            store::save_session_to(temp_dir.path(), &original).expect("save should succeed");

            let result =
                run_codex_resume_with_status_in(temp_dir.path(), "resume-target", None, || {
                    Err(anyhow::anyhow!("resume failed"))
                });

            let reloaded = store::load_session_from(temp_dir.path(), "resume-target")
                .expect("load should succeed")
                .expect("session should exist");
            assert_eq!(
                (
                    result.err().map(|error| error.to_string()),
                    (
                        reloaded.status,
                        reloaded.read_at,
                        reloaded.updated_at,
                        reloaded.tmux_info,
                    ),
                ),
                (Some("resume failed".to_string()), expected)
            );
        }

        #[rstest]
        fn failed_resume_does_not_restore_after_a_hook_update(temp_dir: TempDir) {
            let original = session(Engine::Codex, SessionStatus::Ended);
            store::save_session_to(temp_dir.path(), &original).expect("save should succeed");

            let hook_updated_at = Utc::now();
            let result =
                run_codex_resume_with_status_in(temp_dir.path(), "resume-target", None, || {
                    let mut updated = store::load_session_from(temp_dir.path(), "resume-target")
                        .expect("load should succeed")
                        .expect("session should exist");
                    updated.status = SessionStatus::Running;
                    updated.updated_at = hook_updated_at;
                    store::save_session_to(temp_dir.path(), &updated).expect("save should succeed");
                    Err(anyhow::anyhow!("resume failed"))
                });

            let reloaded = store::load_session_from(temp_dir.path(), "resume-target")
                .expect("load should succeed")
                .expect("session should exist");
            assert_eq!(
                (
                    result.err().map(|error| error.to_string()),
                    reloaded.status,
                    reloaded.updated_at,
                ),
                (
                    Some("resume failed".to_string()),
                    SessionStatus::Running,
                    hook_updated_at,
                )
            );
        }

        #[rstest]
        fn successful_resume_stores_current_tmux_info(temp_dir: TempDir) {
            let original = session(Engine::Codex, SessionStatus::Ended);
            store::save_session_to(temp_dir.path(), &original).expect("save should succeed");
            let current_tmux_info = Some(TmuxInfo {
                session_name: "resumed-session".to_string(),
                window_name: "resumed-window".to_string(),
                window_index: 2,
                pane_id: "%9".to_string(),
            });

            let result = run_codex_resume_with_status_in(
                temp_dir.path(),
                "resume-target",
                current_tmux_info.clone(),
                || Ok(()),
            );
            let reloaded = store::load_session_from(temp_dir.path(), "resume-target")
                .expect("load should succeed")
                .expect("session should exist");

            assert_eq!(
                (
                    result.is_ok(),
                    reloaded.status,
                    reloaded.read_at,
                    reloaded.tmux_info,
                ),
                (true, SessionStatus::Stopped, None, current_tmux_info)
            );
        }
    }

    mod respawn_paused_session_guard_tests {
        use chrono::Utc;
        use std::path::PathBuf;

        use super::*;
        use crate::commands::agent::types::TmuxInfo;

        fn session(status: SessionStatus, tmux_info: Option<TmuxInfo>) -> Session {
            Session {
                session_id: "guard-test".to_string(),
                cwd: PathBuf::from("/tmp/test"),
                transcript_path: None,
                tty: None,
                tmux_info,
                status,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                last_message: None,
                current_tool: None,
                label: None,
                ancestor_session_ids: Vec::new(),
                pending_bg_task_ids: Default::default(),
                pending_agent_task_ids: Default::default(),
                pending_permission_agent_ids: Default::default(),
                read_at: None,
                sweep_signaled: false,
                engine: Engine::Claude,
            }
        }

        fn some_tmux_info() -> TmuxInfo {
            TmuxInfo {
                session_name: "main".to_string(),
                window_name: "editor".to_string(),
                window_index: 0,
                pane_id: "%0".to_string(),
            }
        }

        // Both cases return before shelling out to tmux, so they are safe to
        // run without a real tmux server.
        #[rstest]
        #[case::no_tmux_info(session(SessionStatus::Paused, None), RespawnError::NoTmuxPane)]
        #[case::not_paused(
            session(SessionStatus::Running, Some(some_tmux_info())),
            RespawnError::NotPaused
        )]
        fn guards(#[case] session: Session, #[case] expected: RespawnError) {
            assert_eq!(respawn_paused_session(&session), Err(expected));
        }
    }
}
