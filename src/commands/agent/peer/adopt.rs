use std::path::Path;

use anyhow::Result;
use clap::Args;

use super::{CcError, current_session_id, store};
use crate::commands::agent::types::SessionStatus;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct AdoptArgs {
    /// Session ID to adopt
    pub session_id: String,

    /// Adopt even when the target's current parent is still alive
    #[arg(long)]
    pub force: bool,
}

pub fn run(args: &AdoptArgs) -> Result<()> {
    let caller_id = current_session_id()?;
    adopt_in(
        &store::sessions_dir()?,
        &caller_id,
        &args.session_id,
        args.force,
    )
}

fn adopt_in(sessions_dir: &Path, caller_id: &str, target_id: &str, force: bool) -> Result<()> {
    if target_id == caller_id {
        return Err(CcError::CannotAdoptSelf(target_id.to_string()).into());
    }

    let caller = store::load_session_from(sessions_dir, caller_id)?
        .ok_or_else(|| CcError::SessionNotFound(caller_id.to_string()))?;
    if caller
        .ancestor_session_ids
        .iter()
        .any(|ancestor_id| ancestor_id == target_id)
    {
        return Err(CcError::CannotAdoptAncestor(target_id.to_string()).into());
    }

    let lock = store::lock_session_for_update(sessions_dir, target_id)?;
    let mut target = lock
        .load()?
        .ok_or_else(|| CcError::SessionNotFound(target_id.to_string()))?;
    if target.status == SessionStatus::Ended {
        return Err(CcError::SessionNotFound(target_id.to_string()).into());
    }
    if target.is_direct_child_of(caller_id) {
        return Ok(());
    }

    if !force
        && let Some(parent_id) = target.ancestor_session_ids.last()
        && let Some(parent) = store::load_session_from(sessions_dir, parent_id)?
        && parent.status != SessionStatus::Ended
    {
        return Err(CcError::AdoptRequiresForce {
            session_id: target_id.to_string(),
            parent_id: parent_id.clone(),
        }
        .into());
    }

    target.ancestor_session_ids = caller.ancestor_session_ids;
    target.ancestor_session_ids.push(caller_id.to_string());
    lock.save(&target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::{store, types::Session};
    use chrono::Utc;
    use clap::Parser;
    use rstest::{fixture, rstest};
    use std::path::PathBuf;
    use tempfile::TempDir;

    struct TestSessionStore {
        _temp_dir: TempDir,
        path: PathBuf,
    }

    impl TestSessionStore {
        fn save(&self, session: &Session) {
            store::save_session_to(&self.path, session).expect("session should save");
        }

        fn load(&self, session_id: &str) -> Session {
            store::load_session_from(&self.path, session_id)
                .expect("session should load")
                .expect("session should exist")
        }

        fn adopt(&self, caller_id: &str, target_id: &str, force: bool) -> Result<()> {
            adopt_in(&self.path, caller_id, target_id, force)
        }
    }

    #[fixture]
    fn session_store() -> TestSessionStore {
        let temp_dir = TempDir::new().expect("session directory should be created");
        let path = temp_dir.path().to_path_buf();
        TestSessionStore {
            _temp_dir: temp_dir,
            path,
        }
    }

    fn session(session_id: &str, ancestors: &[&str], status: SessionStatus) -> Session {
        let now = Utc::now();
        Session {
            archived_at: None,
            pr_context: None,
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
            session_id: session_id.to_string(),
            cwd: PathBuf::from("/repo"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status,
            agent_status: None,
            agent_status_clear_pending: false,
            created_at: now,
            updated_at: now,
            last_message: None,
            current_tool: None,
            label: None,
            work_type: None,
            vibe_enabled: false,
            launched_with_vibe: false,
            work_type_pinned: false,
            is_delegated: false,
            ancestor_session_ids: ancestors.iter().map(|id| id.to_string()).collect(),
            pending_bg_task_ids: Default::default(),
            pending_agent_task_ids: Default::default(),
            pending_permission_agent_ids: Default::default(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            engine: Default::default(),
        }
    }

    fn seed_caller_and_target(
        store: &TestSessionStore,
        caller_ancestors: &[&str],
        target_ancestors: &[&str],
    ) {
        store.save(&session("caller", caller_ancestors, SessionStatus::Running));
        store.save(&session("target", target_ancestors, SessionStatus::Running));
    }

    #[test]
    fn adopt_command_parses_target_and_force_flag() {
        #[derive(Parser)]
        struct TestCli {
            #[command(subcommand)]
            command: crate::commands::agent::AgentCommands,
        }

        let actual = TestCli::try_parse_from(["a", "peer", "adopt", "session-x", "--force"])
            .expect("adopt command should parse")
            .command;
        assert!(
            actual
                == crate::commands::agent::AgentCommands::Peer(super::super::PeerCommands::Adopt(
                    AdoptArgs {
                        session_id: "session-x".to_string(),
                        force: true,
                    }
                ))
        );
    }

    #[test]
    fn adopt_rejects_the_calling_session() {
        let store = session_store();

        let error = store
            .adopt("caller", "caller", false)
            .expect_err("adopting the caller would create a cycle");

        assert_eq!(error.to_string(), "Cannot adopt your own session 'caller'");
    }

    #[test]
    fn adopt_rejects_an_ancestor_of_the_caller() {
        let store = session_store();
        seed_caller_and_target(&store, &["ancestor", "parent"], &[]);

        let error = store
            .adopt("caller", "ancestor", false)
            .expect_err("adopting an ancestor would create a cycle");

        assert_eq!(
            error.to_string(),
            "Cannot adopt ancestor session 'ancestor' because it would create a cycle"
        );
    }

    #[rstest]
    #[case::missing(None)]
    #[case::ended(Some(SessionStatus::Ended))]
    fn adopt_rejects_missing_or_ended_targets(
        session_store: TestSessionStore,
        #[case] target_status: Option<SessionStatus>,
    ) {
        session_store.save(&session("caller", &[], SessionStatus::Running));
        if let Some(status) = target_status {
            session_store.save(&session("target", &[], status));
        }

        let error = session_store
            .adopt("caller", "target", true)
            .expect_err("missing and ended targets cannot be adopted");

        assert_eq!(error.to_string(), "Session not found: target");
    }

    #[test]
    fn adopt_succeeds_without_writing_when_target_is_already_a_direct_child() {
        let store = session_store();
        seed_caller_and_target(&store, &["root"], &["root", "caller"]);
        let before = serde_json::to_value(store.load("target")).expect("session should serialize");

        store
            .adopt("caller", "target", false)
            .expect("existing direct child should be accepted");

        let after = serde_json::to_value(store.load("target")).expect("session should serialize");
        assert_eq!(after, before);
    }

    #[test]
    fn adopt_requires_force_when_current_parent_is_alive() {
        let store = session_store();
        seed_caller_and_target(&store, &[], &["old-parent"]);
        store.save(&session("old-parent", &[], SessionStatus::Stopped));

        let error = store
            .adopt("caller", "target", false)
            .expect_err("a non-ended parent requires force");

        assert_eq!(
            error.to_string(),
            "Session 'target' already has living parent 'old-parent'; use --force to adopt it"
        );
    }

    #[rstest]
    #[case::no_current_parent(None, vec![])]
    #[case::missing_parent(None, vec!["old-parent"])]
    #[case::ended_parent(Some(SessionStatus::Ended), vec!["old-parent"])]
    fn adopt_does_not_require_force_when_current_parent_is_not_alive(
        session_store: TestSessionStore,
        #[case] parent_status: Option<SessionStatus>,
        #[case] target_ancestors: Vec<&str>,
    ) {
        seed_caller_and_target(&session_store, &["root"], &target_ancestors);
        if let Some(status) = parent_status {
            session_store.save(&session("old-parent", &[], status));
        }

        session_store
            .adopt("caller", "target", false)
            .expect("missing or ended parent should not block adoption");

        assert_eq!(
            session_store.load("target").ancestor_session_ids,
            vec!["root", "caller"]
        );
    }

    #[test]
    fn adopt_force_reparents_target_and_leaves_descendant_chain_unchanged() {
        let store = session_store();
        seed_caller_and_target(&store, &["root"], &["old-parent"]);
        store.save(&session(
            "descendant",
            &["old-parent", "target"],
            SessionStatus::Running,
        ));
        let descendant_before =
            serde_json::to_value(store.load("descendant")).expect("session should serialize");

        store
            .adopt("caller", "target", true)
            .expect("force should permit reparenting");

        let adopted = store.load("target");
        let sessions = vec![
            store.load("caller"),
            adopted.clone(),
            store.load("descendant"),
        ];
        assert_eq!(
            (
                adopted.ancestor_session_ids.clone(),
                super::super::filter_children(&sessions, "caller")
                    .iter()
                    .map(|session| session.session_id.clone())
                    .collect::<Vec<_>>(),
                adopted.is_direct_child_of("caller"),
                serde_json::to_value(store.load("descendant")).expect("session should serialize"),
            ),
            (
                vec!["root".to_string(), "caller".to_string()],
                vec!["target".to_string()],
                true,
                descendant_before,
            )
        );
    }
}
