use anyhow::{Result, bail};
use clap::Subcommand;

use super::error::CcError;
use super::store;
use crate::shared::config::Config;
use crate::shared::env_var::EnvVars;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum VibeCommands {
    /// Enable vibe mode for the calling session
    On,
    /// Clear the calling session's explicit vibe setting; inherited settings still apply
    Off,
    /// Check whether this session enabled vibe mode or inherited it from an ancestor
    Status,
    /// Grant vibe inheritance to an existing descendant session
    Grant {
        /// ID of the descendant session
        session_id: String,
    },
}

pub fn run(command: &VibeCommands) -> Result<()> {
    match command {
        VibeCommands::On => set(true),
        VibeCommands::Off => set(false),
        VibeCommands::Status => {
            if is_vibe_enabled()? {
                println!("Vibe mode is enabled.");
                Ok(())
            } else {
                bail!("Vibe mode is not enabled.")
            }
        }
        VibeCommands::Grant { session_id } => grant(session_id),
    }
}

/// Returns whether the calling session enabled vibe mode or inherited it from an ancestor.
///
/// An ancestor's flag is inherited only while its session record exists. Returns `false` outside a tracked
/// session or when the caller's session record is missing.
pub fn is_vibe_enabled() -> Result<bool> {
    let Some(session_id) = EnvVars::load().own_session_id() else {
        return Ok(false);
    };

    is_vibe_enabled_for(&store::sessions_dir()?, &session_id)
}

pub(crate) fn ensure_repo_vibe_access(config: &Config, owner: &str, repo: &str) -> Result<()> {
    if !config.is_vibe_allowed_for_repo(owner, repo) {
        bail!("Vibe mode is disabled by configuration for repository {owner}/{repo}.");
    }

    Ok(())
}

fn is_vibe_enabled_for(sessions_dir: &std::path::Path, session_id: &str) -> Result<bool> {
    let Some(session) = store::load_session_from(sessions_dir, session_id)? else {
        return Ok(false);
    };
    if session.vibe_enabled {
        return Ok(true);
    }
    if !session.launched_with_vibe {
        return Ok(false);
    }

    for ancestor_session_id in session.ancestor_session_ids {
        if store::load_session_from(sessions_dir, &ancestor_session_id)?
            .is_some_and(|ancestor| ancestor.vibe_enabled)
        {
            return Ok(true);
        }
    }

    Ok(false)
}

fn set(enabled: bool) -> Result<()> {
    let session_id = EnvVars::load()
        .own_session_id()
        .ok_or(CcError::SelfSessionUnknown)?;
    let sessions_dir = store::sessions_dir()?;
    set_in(&sessions_dir, &session_id, enabled)?;

    if enabled {
        println!("Vibe mode enabled.");
    } else if is_vibe_enabled_for(&sessions_dir, &session_id)? {
        println!("Vibe mode remains enabled by an ancestor.");
    } else {
        println!("Vibe mode disabled.");
    }
    Ok(())
}

fn grant(target_session_id: &str) -> Result<()> {
    let caller_session_id = EnvVars::load()
        .own_session_id()
        .ok_or(CcError::SelfSessionUnknown)?;
    let sessions_dir = store::sessions_dir()?;
    if !is_vibe_enabled_for(&sessions_dir, &caller_session_id)? {
        bail!("Vibe mode is not enabled for the calling session.");
    }

    grant_in(&sessions_dir, &caller_session_id, target_session_id)?;
    println!("Vibe mode granted to session '{target_session_id}'.");
    Ok(())
}

fn grant_in(
    sessions_dir: &std::path::Path,
    caller_session_id: &str,
    target_session_id: &str,
) -> Result<()> {
    let lock = store::lock_session_for_update(sessions_dir, target_session_id)?;
    let mut session = lock
        .load()?
        .ok_or_else(|| CcError::SessionNotFound(target_session_id.to_string()))?;
    if !session
        .ancestor_session_ids
        .iter()
        .any(|ancestor_id| ancestor_id == caller_session_id)
    {
        bail!("Session '{caller_session_id}' is not an ancestor of '{target_session_id}'.");
    }

    session.launched_with_vibe = true;
    lock.save(&session)?;
    Ok(())
}

fn set_in(sessions_dir: &std::path::Path, session_id: &str, enabled: bool) -> Result<()> {
    let lock = store::lock_session_for_update(sessions_dir, session_id)?;
    let mut session = lock
        .load()?
        .ok_or_else(|| CcError::SessionNotFound(session_id.to_string()))?;
    session.vibe_enabled = enabled;
    lock.save(&session)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Commands};
    use crate::commands::agent::types::{Engine, Session, SessionStatus};
    use crate::shared::config::Config;
    use chrono::Utc;
    use clap::Parser;
    use indoc::indoc;
    use rstest::{fixture, rstest};
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use tempfile::TempDir;

    struct TestStore {
        _temp_dir: TempDir,
        cache_home: PathBuf,
        sessions_dir: PathBuf,
    }

    #[fixture]
    fn test_store() -> TestStore {
        let temp_dir = TempDir::new().expect("temp dir creation should succeed");
        let cache_home = temp_dir.path().to_path_buf();
        let sessions_dir = cache_home.join("armyknife/cc/sessions");
        TestStore {
            _temp_dir: temp_dir,
            cache_home,
            sessions_dir,
        }
    }

    fn session(session_id: &str, ancestors: &[&str], vibe_enabled: bool) -> Session {
        Session {
            archived_at: None,
            pr_context: None,
            session_id: session_id.to_string(),
            crit_urls: Vec::new(),
            pending_human_review_ids: BTreeSet::new(),
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
            work_type: None,
            vibe_enabled,
            launched_with_vibe: false,
            work_type_pinned: false,
            is_delegated: false,
            ancestor_session_ids: ancestors.iter().map(|id| (*id).to_string()).collect(),
            pending_bg_task_ids: BTreeSet::new(),
            pending_agent_task_ids: BTreeSet::new(),
            pending_permission_agent_ids: BTreeSet::new(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            engine: Engine::Claude,
        }
    }

    #[test]
    fn vibe_commands_parse_from_the_cli() {
        let cli =
            Cli::try_parse_from(["a", "agent", "vibe", "on"]).expect("vibe command should parse");

        assert!(
            cli.command == Commands::Agent(super::super::AgentCommands::Vibe(VibeCommands::On))
        );
    }

    #[test]
    fn vibe_grant_command_parses_a_session_id_from_the_cli() {
        let cli = Cli::try_parse_from(["a", "agent", "vibe", "grant", "child-session"])
            .expect("vibe grant command should parse");
        let actual_session_id = match cli.command {
            Commands::Agent(super::super::AgentCommands::Vibe(VibeCommands::Grant {
                session_id,
            })) => Some(session_id),
            _ => None,
        };

        assert_eq!(actual_session_id, Some("child-session".to_string()));
    }

    #[rstest]
    #[case::allowed(
        indoc! {"
            orgs:
              example-org:
                vibe: true
        "},
        Ok(())
    )]
    #[case::denied_by_default(
        "{}",
        Err("Vibe mode is disabled by configuration for repository example-org/example-repo.")
    )]
    fn repo_vibe_access_uses_effective_config(
        #[case] yaml: &str,
        #[case] expected: std::result::Result<(), &str>,
    ) {
        let config: Config = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(
            ensure_repo_vibe_access(&config, "example-org", "example-repo")
                .map_err(|error| error.to_string()),
            expected.map_err(str::to_owned),
        );
    }

    #[rstest]
    fn commands_toggle_only_the_calling_session_and_status_follows_ancestors(
        test_store: TestStore,
    ) {
        let root = session("root", &[], false);
        let child = session("child", &["root"], false);
        store::save_session_to(&test_store.sessions_dir, &root).expect("root should save");
        store::save_session_to(&test_store.sessions_dir, &child).expect("child should save");
        set_in(&test_store.sessions_dir, "child", true).expect("on should save");
        let enabled = is_vibe_enabled_for(&test_store.sessions_dir, "child")
            .expect("vibe status should load");
        let child_enabled = store::load_session_from(&test_store.sessions_dir, "child")
            .expect("child should load")
            .expect("child should exist")
            .vibe_enabled;
        let root_enabled = store::load_session_from(&test_store.sessions_dir, "root")
            .expect("root should load")
            .expect("root should exist")
            .vibe_enabled;
        set_in(&test_store.sessions_dir, "child", false).expect("off should save");
        let disabled = is_vibe_enabled_for(&test_store.sessions_dir, "child")
            .expect("vibe status should load");

        let actual = (enabled, disabled, child_enabled, root_enabled);

        assert_eq!(actual, (true, false, true, false));
    }

    #[rstest]
    #[case::caller_enabled_directly(false)]
    #[case::caller_inherits_from_ancestor(true)]
    fn vibe_enabled_ancestor_can_grant_an_existing_descendant(
        test_store: TestStore,
        #[case] caller_inherits_vibe: bool,
    ) {
        let mut caller = session(
            "caller-session",
            if caller_inherits_vibe {
                &["root-session"]
            } else {
                &[]
            },
            !caller_inherits_vibe,
        );
        caller.launched_with_vibe = caller_inherits_vibe;
        let child_ancestors = if caller_inherits_vibe {
            &["root-session", "caller-session"][..]
        } else {
            &["caller-session"][..]
        };
        let child = session("child-session", child_ancestors, false);
        if caller_inherits_vibe {
            let root = session("root-session", &[], true);
            store::save_session_to(&test_store.sessions_dir, &root).expect("root should save");
        }
        store::save_session_to(&test_store.sessions_dir, &caller).expect("caller should save");
        store::save_session_to(&test_store.sessions_dir, &child).expect("child should save");
        let cache_home = test_store
            .cache_home
            .to_str()
            .expect("cache path should be valid UTF-8");

        let grant_result = with_session_env(cache_home, Some("caller-session"), || {
            run(&VibeCommands::Grant {
                session_id: "child-session".to_string(),
            })
            .map_err(|error| error.to_string())
        });
        let status_result = with_session_env(cache_home, Some("child-session"), || {
            run(&VibeCommands::Status).map_err(|error| error.to_string())
        });
        let saved_child = store::load_session_from(&test_store.sessions_dir, "child-session")
            .expect("child should load")
            .map(|session| serde_json::to_value(session).expect("child should serialize"));
        let mut expected_child = child;
        expected_child.launched_with_vibe = true;
        let expected_child =
            Some(serde_json::to_value(expected_child).expect("expected child should serialize"));

        assert_eq!(
            (grant_result, status_result, saved_child),
            (Ok(()), Ok(()), expected_child),
        );
    }

    #[rstest]
    #[case::caller_is_not_an_ancestor(
        true,
        false,
        true,
        "Session 'caller-session' is not an ancestor of 'child-session'."
    )]
    #[case::caller_vibe_is_disabled(
        false,
        true,
        true,
        "Vibe mode is not enabled for the calling session."
    )]
    #[case::target_session_is_missing(true, true, false, "Session not found: child-session")]
    fn invalid_vibe_grants_fail_without_changing_the_target(
        test_store: TestStore,
        #[case] caller_vibe_enabled: bool,
        #[case] caller_is_ancestor: bool,
        #[case] target_exists: bool,
        #[case] expected_error: &str,
    ) {
        let caller = session("caller-session", &[], caller_vibe_enabled);
        let ancestors = if caller_is_ancestor {
            &["caller-session"][..]
        } else {
            &["other-session"][..]
        };
        let target = session("child-session", ancestors, false);
        store::save_session_to(&test_store.sessions_dir, &caller).expect("caller should save");
        if target_exists {
            store::save_session_to(&test_store.sessions_dir, &target).expect("child should save");
        }
        let cache_home = test_store
            .cache_home
            .to_str()
            .expect("cache path should be valid UTF-8");
        let before = store::load_session_from(&test_store.sessions_dir, "child-session")
            .expect("child should load")
            .map(|session| serde_json::to_value(session).expect("child should serialize"));

        let result = with_session_env(cache_home, Some("caller-session"), || {
            run(&VibeCommands::Grant {
                session_id: "child-session".to_string(),
            })
            .map_err(|error| error.to_string())
        });
        let after = store::load_session_from(&test_store.sessions_dir, "child-session")
            .expect("child should load")
            .map(|session| serde_json::to_value(session).expect("child should serialize"));

        assert_eq!((result, after), (Err(expected_error.to_string()), before));
    }

    #[rstest]
    fn disabling_the_enabling_ancestor_disables_descendant_status(test_store: TestStore) {
        let root = session("root", &[], false);
        let mut child = session("child", &["root"], false);
        child.launched_with_vibe = true;
        store::save_session_to(&test_store.sessions_dir, &root).expect("root should save");
        store::save_session_to(&test_store.sessions_dir, &child).expect("child should save");
        let cache_home = test_store
            .cache_home
            .to_str()
            .expect("cache path should be valid UTF-8");

        let run_for = |session_id, command| {
            with_session_env(cache_home, Some(session_id), || run(command).is_ok())
        };
        let status_command_succeeds_for = |session_id| {
            with_session_env(cache_home, Some(session_id), || {
                run(&VibeCommands::Status).is_ok()
            })
        };

        let enabled_command_succeeded = run_for("root", &VibeCommands::On);
        let child_status_succeeds = status_command_succeeds_for("child");
        let child_off_command_succeeded = run_for("child", &VibeCommands::Off);
        let child_status_still_succeeds = status_command_succeeds_for("child");
        let disabled_command_succeeded = run_for("root", &VibeCommands::Off);
        let child_status_fails = !status_command_succeeds_for("child");

        assert_eq!(
            (
                enabled_command_succeeded,
                child_status_succeeds,
                child_off_command_succeeded,
                child_status_still_succeeds,
                disabled_command_succeeded,
                child_status_fails,
            ),
            (true, true, true, true, true, true),
        );
    }

    #[rstest]
    fn vibe_status_uses_the_full_ancestor_chain(test_store: TestStore) {
        let root = session("root", &[], true);
        let parent = session("parent", &["root"], false);
        let mut child = session("child", &["root", "parent"], false);
        child.launched_with_vibe = true;
        store::save_session_to(&test_store.sessions_dir, &root).expect("root should save");
        store::save_session_to(&test_store.sessions_dir, &parent).expect("parent should save");
        store::save_session_to(&test_store.sessions_dir, &child).expect("child should save");
        let cache_home = test_store
            .cache_home
            .to_str()
            .expect("cache path should be valid UTF-8");
        let actual = with_session_env(cache_home, Some("child"), || {
            is_vibe_enabled().expect("vibe status should load")
        });

        assert!(actual);
    }

    #[rstest]
    #[case::launched_child_inherits_active_ancestor(true, true, true)]
    #[case::ordinary_child_does_not_inherit(true, false, false)]
    #[case::launched_child_requires_active_ancestor(false, true, false)]
    fn status_inherits_only_for_vibe_launched_sessions(
        test_store: TestStore,
        #[case] ancestor_vibe_enabled: bool,
        #[case] launched_with_vibe: bool,
        #[case] expected: bool,
    ) {
        let root = session("root", &[], ancestor_vibe_enabled);
        let mut child = session("child", &["root"], false);
        child.launched_with_vibe = launched_with_vibe;
        store::save_session_to(&test_store.sessions_dir, &root).expect("root should save");
        store::save_session_to(&test_store.sessions_dir, &child).expect("child should save");
        let cache_home = test_store
            .cache_home
            .to_str()
            .expect("cache path should be valid UTF-8");

        let actual = with_session_env(cache_home, Some("child"), || {
            is_vibe_enabled().expect("vibe status should load")
        });

        assert_eq!(actual, expected);
    }

    #[rstest]
    fn missing_ancestor_record_does_not_enable_vibe_mode(test_store: TestStore) {
        let child = session("child", &["removed-root"], false);
        store::save_session_to(&test_store.sessions_dir, &child).expect("child should save");

        let actual = is_vibe_enabled_for(&test_store.sessions_dir, "child")
            .expect("vibe status should load");

        assert!(!actual);
    }

    #[rstest]
    fn session_without_an_id_is_not_vibe_enabled(test_store: TestStore) {
        let root = session("root", &[], true);
        store::save_session_to(&test_store.sessions_dir, &root).expect("root should save");

        let actual = temp_env::with_vars(
            [
                (
                    "XDG_CACHE_HOME",
                    Some(
                        test_store
                            .cache_home
                            .to_str()
                            .expect("cache path should be valid UTF-8"),
                    ),
                ),
                ("ARMYKNIFE_SESSION_ID", None::<&str>),
                ("CODEX_SESSION_ID", None::<&str>),
                ("ARMYKNIFE_ANCESTOR_SESSION_IDS", Some("root")),
            ],
            || is_vibe_enabled().expect("vibe status should load"),
        );

        assert!(!actual);
    }

    fn with_session_env<T>(cache_home: &str, session_id: Option<&str>, f: impl FnOnce() -> T) -> T {
        temp_env::with_vars(
            [
                ("XDG_CACHE_HOME", Some(cache_home)),
                ("ARMYKNIFE_SESSION_ID", session_id),
                ("CODEX_SESSION_ID", None::<&str>),
            ],
            f,
        )
    }
}
