use anyhow::{Result, ensure};
use clap::{Args, Subcommand};

use super::error::CcError;
use super::store;
use super::types::{AgentStatus, AgentStatusKind};
use crate::shared::env_var::EnvVars;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum StatusCommands {
    /// Set the calling session's reason for stopping
    Set(SetArgs),
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct SetArgs {
    /// Why the agent stopped
    #[arg(value_enum)]
    pub kind: AgentStatusKind,

    /// One-line description of what is needed
    pub note: String,
}

pub fn run(command: &StatusCommands) -> Result<()> {
    match command {
        StatusCommands::Set(args) => set(args),
    }
}

fn set(args: &SetArgs) -> Result<()> {
    let session_id = EnvVars::load()
        .own_session_id()
        .ok_or(CcError::SelfSessionUnknown)?;
    let sessions_dir = store::sessions_dir()?;
    set_status_in(
        &sessions_dir,
        &session_id,
        AgentStatus {
            kind: args.kind,
            note: args.note.clone(),
        },
    )
}

fn set_status_in(
    sessions_dir: &std::path::Path,
    session_id: &str,
    status: AgentStatus,
) -> Result<()> {
    ensure_single_line(&status.note)?;
    let lock = store::lock_session_for_update(sessions_dir, session_id)?;
    let mut session = lock
        .load()?
        .ok_or_else(|| CcError::SessionNotFound(session_id.to_string()))?;
    session.agent_status = Some(status);
    lock.save(&session)
}

fn ensure_single_line(note: &str) -> Result<()> {
    ensure!(
        !note.contains('\n') && !note.contains('\r'),
        "status note must be a single line"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::types::{Engine, Session, SessionStatus};
    use chrono::Utc;
    use clap::Parser;
    use indoc::indoc;
    use rstest::rstest;
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: super::super::AgentCommands,
    }

    fn session(session_id: &str, engine: Engine) -> Session {
        Session {
            session_id: session_id.to_string(),
            work_type: None,
            work_type_pinned: false,
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: PathBuf::from("/tmp/test"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            agent_status: None,
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
    fn set_command_parses_kind_and_note() {
        let cli = TestCli::try_parse_from(["a", "status", "set", "wait", "waiting for CI"])
            .expect("status set arguments should parse");

        assert!(
            cli.command
                == super::super::AgentCommands::Status(StatusCommands::Set(SetArgs {
                    kind: AgentStatusKind::Wait,
                    note: "waiting for CI".to_string(),
                }))
        );
    }

    #[rstest]
    #[case::single_line("waiting for CI", true)]
    #[case::empty("", true)]
    #[case::line_feed(indoc! {"
        first
        second
    "}, false)]
    #[case::carriage_return("first\rsecond", false)]
    fn status_note_is_one_line(#[case] note: &str, #[case] expected: bool) {
        assert_eq!(ensure_single_line(note).is_ok(), expected);
    }

    #[test]
    fn set_status_persists_to_the_calling_codex_session() {
        let temp_dir = TempDir::new().expect("temp dir creation should succeed");
        let xdg_cache_home = temp_dir
            .path()
            .to_str()
            .expect("temp dir path should be valid UTF-8");
        let sessions_dir = temp_dir.path().join("armyknife/cc/sessions");
        let codex_session = session("codex-session", Engine::Codex);
        let other_session = session("other-session", Engine::Claude);
        store::save_session_to(&sessions_dir, &codex_session).expect("session should save");
        store::save_session_to(&sessions_dir, &other_session).expect("session should save");
        let expected = AgentStatus {
            kind: AgentStatusKind::Decide,
            note: "choose a schedule".to_string(),
        };

        temp_env::with_vars(
            [
                ("XDG_CACHE_HOME", Some(xdg_cache_home)),
                ("ARMYKNIFE_SESSION_ID", None::<&str>),
                ("CODEX_SESSION_ID", Some("codex-session")),
            ],
            || {
                run(&StatusCommands::Set(SetArgs {
                    kind: expected.kind,
                    note: expected.note.clone(),
                }))
                .expect("status should save");
            },
        );

        let actual = (
            store::load_session_from(&sessions_dir, "codex-session")
                .expect("Codex session should load")
                .expect("Codex session should exist")
                .agent_status,
            store::load_session_from(&sessions_dir, "other-session")
                .expect("other session should load")
                .expect("other session should exist")
                .agent_status,
        );

        assert_eq!(actual, (Some(expected), None));
    }
}
