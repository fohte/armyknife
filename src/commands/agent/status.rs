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

    let status = AgentStatus {
        kind: args.kind.clone(),
        note: args.note.clone(),
    };
    ensure_single_line(&status.note)?;

    let env_vars = EnvVars::load();
    // Codex shares the root session ID with subagent threads, so their statuses
    // must not overwrite the root record.
    if is_codex_subagent_thread(&env_vars) {
        return Ok(());
    }

    let session_id = env_vars
        .own_session_id()
        .ok_or(CcError::SelfSessionUnknown)?;
    let sessions_dir = store::sessions_dir()?;
    set_status_in(&sessions_dir, &session_id, status)
}

fn is_codex_subagent_thread(env_vars: &EnvVars) -> bool {
    matches!(
        (
            env_vars.codex_session_id.as_deref(),
            env_vars.codex_thread_id.as_deref(),
        ),
        (Some(session_id), Some(thread_id)) if session_id != thread_id
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
            archived_at: None,
            pr_context: None,
            session_id: session_id.to_string(),
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            child_launch: Default::default(),
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
    #[case::codex_root_without_thread(Engine::Codex, None, Some("codex-session"), None)]
    #[case::codex_root_with_matching_thread_id(
        Engine::Codex,
        None,
        Some("codex-session"),
        Some("codex-session")
    )]
    #[case::empty_thread_id(Engine::Codex, None, Some("codex-session"), Some(""))]
    #[case::claude_session(Engine::Claude, Some("codex-session"), None, None)]
    #[case::thread_id_without_session_id(
        Engine::Claude,
        Some("codex-session"),
        None,
        Some("codex-subagent-thread")
    )]
    fn set_status_persists_to_the_calling_session(
        configured_status_home: TempDir,
        #[case] engine: Engine,
        #[case] armyknife_session_id: Option<&str>,
        #[case] codex_session_id: Option<&str>,
        #[case] codex_thread_id: Option<&str>,
    ) {
        let temp_dir = TempDir::new().expect("temp dir creation should succeed");
        let xdg_cache_home = temp_dir
            .path()
            .to_str()
            .expect("temp dir path should be valid UTF-8");
        let sessions_dir = temp_dir.path().join("armyknife/cc/sessions");
        let mut calling_session = session("codex-session", engine);
        calling_session.agent_status_clear_pending = true;
        let other_session = session("other-session", Engine::Claude);
        store::save_session_to(&sessions_dir, &calling_session).expect("session should save");
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
                ("ARMYKNIFE_SESSION_ID", armyknife_session_id),
                ("CODEX_SESSION_ID", codex_session_id),
                ("CODEX_THREAD_ID", codex_thread_id),
            ],
            || {
                run(&StatusCommands::Set(SetArgs {
                    kind: expected.kind.clone(),
                    note: expected.note.clone(),
                }))
                .expect("status should save");
            },
        );

        let calling_session = store::load_session_from(&sessions_dir, "codex-session")
            .expect("calling session should load")
            .expect("calling session should exist");
        let actual = (
            (
                calling_session.agent_status,
                calling_session.agent_status_clear_pending,
            ),
            store::load_session_from(&sessions_dir, "other-session")
                .expect("other session should load")
                .expect("other session should exist")
                .agent_status,
        );

        assert_eq!(actual, ((Some(expected), false), None));
    }

    #[rstest]
    fn set_from_codex_subagent_leaves_the_shared_session_unchanged(
        configured_status_home: TempDir,
    ) {
        let temp_dir = TempDir::new().expect("temp dir creation should succeed");
        let xdg_cache_home = temp_dir
            .path()
            .to_str()
            .expect("temp dir path should be valid UTF-8");
        let sessions_dir = temp_dir.path().join("armyknife/cc/sessions");
        let mut parent_session = session("codex-session", Engine::Codex);
        parent_session.agent_status = Some(AgentStatus {
            kind: "needs_attention".to_string(),
            note: "parent status".to_string(),
        });
        parent_session.agent_status_clear_pending = true;
        store::save_session_to(&sessions_dir, &parent_session).expect("session should save");
        let before = store::serialize_session(
            &store::load_session_from(&sessions_dir, "codex-session")
                .expect("session should load")
                .expect("session should exist"),
        )
        .expect("session should serialize");
        let before_revision =
            store::session_store_revision_from(&sessions_dir).expect("store revision should load");
        let xdg_config_home = configured_status_home
            .path()
            .to_str()
            .expect("config home path should be valid UTF-8");

        temp_env::with_vars(
            [
                ("XDG_CACHE_HOME", Some(xdg_cache_home)),
                ("XDG_CONFIG_HOME", Some(xdg_config_home)),
                ("ARMYKNIFE_SESSION_ID", Some("codex-session")),
                ("CODEX_SESSION_ID", Some("codex-session")),
                ("CODEX_THREAD_ID", Some("codex-subagent-thread")),
            ],
            || {
                run(&StatusCommands::Set(SetArgs {
                    kind: "needs_attention".to_string(),
                    note: "child status".to_string(),
                }))
                .expect("subagent status should succeed");
            },
        );

        let after = store::serialize_session(
            &store::load_session_from(&sessions_dir, "codex-session")
                .expect("session should load")
                .expect("session should exist"),
        )
        .expect("session should serialize");
        let after_revision =
            store::session_store_revision_from(&sessions_dir).expect("store revision should load");

        assert_eq!((after, after_revision), (before, before_revision));
    }
}
