use anyhow::{Result, bail};
use clap::Args;
use thiserror::Error;

use super::types::{Session, SessionStatus, TMUX_SESSION_OPTION};
use crate::infra::{process, tmux};
use crate::shared::command::find_command_path;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ResumeArgs {
    /// Claude Code session ID to resume. When omitted, the session ID is read from
    /// the current tmux pane's `@armyknife-last-claude-code-session-id` user option.
    pub session_id: Option<String>,
}

/// Runs the resume command.
/// If a session ID argument is provided, resumes that session directly.
/// Otherwise, reads the session ID from the current tmux pane's user option.
pub fn run(args: &ResumeArgs) -> Result<()> {
    let session_id = match args.session_id.as_deref() {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => resolve_session_id_from_pane()?,
    };

    let claude_path = find_command_path("claude")
        .ok_or_else(|| anyhow::anyhow!("Could not find 'claude' command in PATH"))?;

    let err = process::exec_replace(&claude_path, ["--resume", &session_id]);
    bail!("Failed to exec claude: {}", err)
}

fn resolve_session_id_from_pane() -> Result<String> {
    let pane_id = current_pane_id()?;
    tmux::get_pane_option(&pane_id, TMUX_SESSION_OPTION)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No Claude Code session ID found for pane {} (option '{}' not set or empty)",
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
    match std::env::var("TMUX_PANE") {
        Ok(value) if !value.is_empty() => Ok(value),
        _ => bail!("Not running inside a tmux pane: $TMUX_PANE is not set"),
    }
}

/// Programs a `Paused` session's pane may be sitting at for its respawn to
/// be safe. Anything else means a foreground program is running that a
/// blind `respawn-pane -k` would kill.
const SHELL_COMMANDS: &[&str] = &["zsh", "bash", "fish", "sh", "dash"];

/// Failure modes of [`respawn_paused_session`]. Both the TUI's resume key
/// and `a cc wake` render these as user-facing messages.
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

/// Replaces a paused session's pane's root process with `a cc resume`
/// wrapped in the user's login shell, so `claude --resume` restarts in it.
/// Does not focus the pane -- callers that want that (the TUI) do it
/// themselves afterward, since a resume triggered from another session must
/// not steal the user's tmux focus.
///
/// The session actually resumed is whichever one is recorded on the pane's
/// `TMUX_SESSION_OPTION`, not necessarily `session.session_id` -- callers
/// that can't guarantee the two are in sync must verify this themselves
/// (see `wake::check_pane_matches_target`).
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

/// Wraps `a cc resume` in the user's login shell so that when claude exits
/// normally, control returns to a shell prompt instead of tmux closing the
/// pane (`respawn-pane` replaces the pane's root process).
///
/// `-i` is required on the outer shell: `a cc resume` looks up `claude` in
/// $PATH via `find_command_path`, and many users only extend $PATH in their
/// interactive rc file (e.g. `.zshrc`). Running without `-i` would inherit
/// tmux's pre-rc $PATH and fail to locate `claude`.
fn build_resume_command() -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_else(|| "a".to_string());
    let inner = shlex::try_join([exe.as_str(), "cc", "resume"]).ok()?;
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

    mod respawn_paused_session_guard_tests {
        use chrono::Utc;
        use std::path::PathBuf;

        use super::*;
        use crate::commands::cc::types::TmuxInfo;

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
