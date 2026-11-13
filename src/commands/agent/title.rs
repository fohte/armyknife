use anyhow::Result;
use clap::{Args, Subcommand};

use super::error::CcError;
use super::session_title;
use crate::shared::env_var::EnvVars;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum TitleCommands {
    /// Set the calling session's title; pass an empty string to restore its automatic title
    Set(SetArgs),
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct SetArgs {
    /// New title (empty restores the automatic title)
    pub title: String,
}

pub fn run(command: &TitleCommands) -> Result<()> {
    match command {
        TitleCommands::Set(args) => set(&args.title),
    }
}

fn set(title: &str) -> Result<()> {
    let session_id = EnvVars::load()
        .own_session_id()
        .ok_or(CcError::SelfSessionUnknown)?;
    session_title::set_session_title(&session_id, title)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;

    use clap::Parser;
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use crate::commands::agent::types::{Engine, Session, SessionStatus};
    use crate::commands::agent::{AgentCommands, store};

    use super::{SetArgs, TitleCommands, run};

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: AgentCommands,
    }

    #[rstest::rstest]
    #[case::title("New session title", "New session title")]
    #[case::empty("", "")]
    fn parses_title_set(#[case] title: &str, #[case] expected: &str) {
        let parsed = TestCli::try_parse_from(["a", "title", "set", title])
            .expect("title set command should parse");
        let AgentCommands::Title(TitleCommands::Set(args)) = parsed.command else {
            panic!("title set should select the title command");
        };

        assert_eq!(args.title, expected);
    }

    #[fixture]
    fn cache_home() -> TempDir {
        tempfile::tempdir().expect("temporary cache directory should be created")
    }

    fn session(session_id: &str, label: &str) -> Session {
        Session {
            session_id: session_id.to_string(),
            archived_at: None,
            pr_context: None,
            crit_urls: Vec::new(),
            pending_human_review_ids: BTreeSet::new(),
            cwd: PathBuf::from("/fixture/project"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            agent_status: None,
            agent_status_clear_pending: false,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            last_message: None,
            current_tool: None,
            label: Some(label.to_string()),
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            child_launch: Default::default(),
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: BTreeSet::new(),
            pending_agent_task_ids: BTreeSet::new(),
            pending_permission_agent_ids: BTreeSet::new(),
            pending_permission_request_ids: BTreeMap::new(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            engine: Engine::Claude,
            model: None,
            effort: None,
        }
    }

    #[rstest]
    #[case::sets_trimmed_title("  New title  ", Some("New title"))]
    #[case::empty_restores_automatic_title("", None)]
    fn run_updates_only_the_calling_session(
        #[case] title: &str,
        #[case] expected_label: Option<&str>,
        cache_home: TempDir,
    ) {
        let sessions_dir = cache_home.path().join("armyknife/cc/sessions");
        store::save_session_to(&sessions_dir, &session("self-session", "Old title"))
            .expect("calling session should save");
        store::save_session_to(&sessions_dir, &session("other-session", "Other title"))
            .expect("other session should save");

        temp_env::with_vars(
            [
                (
                    "XDG_CACHE_HOME",
                    Some(
                        cache_home
                            .path()
                            .to_str()
                            .expect("cache path should be UTF-8"),
                    ),
                ),
                ("ARMYKNIFE_SESSION_ID", Some("self-session")),
                ("CODEX_SESSION_ID", None::<&str>),
            ],
            || {
                run(&TitleCommands::Set(SetArgs {
                    title: title.to_string(),
                }))
                .expect("title command should succeed");
            },
        );

        let calling_session = store::load_session_from(&sessions_dir, "self-session")
            .expect("calling session should load")
            .expect("calling session should exist");
        let other_session = store::load_session_from(&sessions_dir, "other-session")
            .expect("other session should load")
            .expect("other session should exist");
        assert_eq!(
            (calling_session.label, other_session.label),
            (
                expected_label.map(str::to_string),
                Some("Other title".to_string()),
            ),
        );
    }
}
