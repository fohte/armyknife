use anyhow::{Result, ensure};
use clap::{Args, Subcommand};

use super::error::CcError;
use super::store;
use super::types::AgentStatus;
use crate::shared::config::{self, AgentConfig};
use crate::shared::env_var::EnvVars;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum StatusCommands {
    /// Set the calling session's reason for stopping
    Set(SetArgs),
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct SetArgs {
    /// Why the agent stopped
    #[arg(value_name = "KIND")]
    pub kind: String,

    /// One-line description of what is needed
    pub note: String,
}

pub fn run(command: &StatusCommands) -> Result<()> {
    let agent_config = config::load_config()?.agent;
    match command {
        StatusCommands::Set(args) => set(args, &agent_config),
    }
}

fn set(args: &SetArgs, agent_config: &AgentConfig) -> Result<()> {
    ensure!(
        agent_config.statuses.contains_key(&args.kind),
        "unknown agent status kind `{}`; configured kinds: {}",
        args.kind,
        configured_kind_names(agent_config),
    );

    let session_id = EnvVars::load()
        .own_session_id()
        .ok_or(CcError::SelfSessionUnknown)?;
    let sessions_dir = store::sessions_dir()?;
    set_status_in(
        &sessions_dir,
        &session_id,
        AgentStatus {
            kind: args.kind.clone(),
            note: args.note.clone(),
        },
    )
}

fn configured_kind_names(agent_config: &AgentConfig) -> String {
    let mut kinds = agent_config
        .statuses
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    kinds.sort_unstable();
    if kinds.is_empty() {
        "(none)".to_string()
    } else {
        kinds.join(", ")
    }
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
    session.agent_status_clear_pending = false;
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
    use rstest::{fixture, rstest};
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
            vibe_enabled: false,
            is_delegated: false,
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: PathBuf::from("/tmp/test"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            agent_status: None,
            agent_status_clear_pending: false,
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
            engine,
        }
    }

    #[fixture]
    fn configured_status_home() -> TempDir {
        let home = TempDir::new().expect("config home should be created");
        let config_dir = home.path().join("armyknife");
        std::fs::create_dir_all(&config_dir).expect("config directory should be created");
        std::fs::write(
            config_dir.join("statuses.yaml"),
            indoc! {"
                agent:
                  statuses:
                    needs_attention: { state: user }
            "},
        )
        .expect("config should be written");
        home
    }

    #[test]
    fn set_command_parses_kind_and_note() {
        let cli = TestCli::try_parse_from([
            "a",
            "status",
            "set",
            "awaiting_build",
            "waiting for a build",
        ])
        .expect("status set arguments should parse");

        assert!(
            cli.command
                == super::super::AgentCommands::Status(StatusCommands::Set(SetArgs {
                    kind: "awaiting_build".to_string(),
                    note: "waiting for a build".to_string(),
                }))
        );
    }

    #[rstest]
    fn set_rejects_kinds_missing_from_configuration(configured_status_home: TempDir) {
        let config_home = configured_status_home
            .path()
            .to_str()
            .expect("config home path should be valid UTF-8");
        let error = temp_env::with_vars([("XDG_CONFIG_HOME", Some(config_home))], || {
            run(&StatusCommands::Set(SetArgs {
                kind: "old_name".to_string(),
                note: "review the proposal".to_string(),
            }))
        })
        .expect_err("unconfigured status names should be rejected");
        assert_eq!(
            error.to_string(),
            "unknown agent status kind `old_name`; configured kinds: needs_attention",
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

    #[rstest]
    fn set_status_persists_to_the_calling_codex_session(configured_status_home: TempDir) {
        let temp_dir = TempDir::new().expect("temp dir creation should succeed");
        let xdg_cache_home = temp_dir
            .path()
            .to_str()
            .expect("temp dir path should be valid UTF-8");
        let sessions_dir = temp_dir.path().join("armyknife/cc/sessions");
        let mut codex_session = session("codex-session", Engine::Codex);
        codex_session.agent_status_clear_pending = true;
        let other_session = session("other-session", Engine::Claude);
        store::save_session_to(&sessions_dir, &codex_session).expect("session should save");
        store::save_session_to(&sessions_dir, &other_session).expect("session should save");
        let expected = AgentStatus {
            kind: "needs_attention".to_string(),
            note: "choose a schedule".to_string(),
        };
        let xdg_config_home = configured_status_home
            .path()
            .to_str()
            .expect("config home path should be valid UTF-8");

        temp_env::with_vars(
            [
                ("XDG_CACHE_HOME", Some(xdg_cache_home)),
                ("XDG_CONFIG_HOME", Some(xdg_config_home)),
                ("ARMYKNIFE_SESSION_ID", None::<&str>),
                ("CODEX_SESSION_ID", Some("codex-session")),
            ],
            || {
                run(&StatusCommands::Set(SetArgs {
                    kind: expected.kind.clone(),
                    note: expected.note.clone(),
                }))
                .expect("status should save");
            },
        );

        let codex_session = store::load_session_from(&sessions_dir, "codex-session")
            .expect("Codex session should load")
            .expect("Codex session should exist");
        let actual = (
            (
                codex_session.agent_status,
                codex_session.agent_status_clear_pending,
            ),
            store::load_session_from(&sessions_dir, "other-session")
                .expect("other session should load")
                .expect("other session should exist")
                .agent_status,
        );

        assert_eq!(actual, ((Some(expected), false), None));
    }
}
