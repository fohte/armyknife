use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use clap::Args;

use super::{CcError, current_session_id, store};
use crate::commands::agent::types::{Session, SessionStatus};

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

    let lock = store::lock_session_for_update(sessions_dir, target_id)?;
    let caller = store::load_session_from(sessions_dir, caller_id)?
        .ok_or_else(|| CcError::SessionNotFound(caller_id.to_string()))?;
    if has_ancestor(sessions_dir, &caller, caller_id, target_id)? {
        return Err(CcError::CannotAdoptAncestor(target_id.to_string()).into());
    }

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
    target.launched_with_vibe = false;
    lock.save(&target)
}

fn has_ancestor(
    sessions_dir: &Path,
    caller: &Session,
    caller_id: &str,
    target_id: &str,
) -> Result<bool> {
    if caller
        .ancestor_session_ids
        .iter()
        .any(|ancestor_id| ancestor_id == target_id)
    {
        return Ok(true);
    }

    // Adopt leaves descendant chains unchanged, so resolve their parent links through the store.
    let mut visited = HashSet::from([caller_id.to_string()]);
    let mut parent_id = caller.ancestor_session_ids.last().cloned();
    while let Some(current_id) = parent_id {
        if current_id == target_id {
            return Ok(true);
        }
        if !visited.insert(current_id.clone()) {
            break;
        }
        parent_id = store::load_session_from(sessions_dir, &current_id)?
            .and_then(|parent| parent.ancestor_session_ids.last().cloned());
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::store;
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
            child_launch: Default::default(),
            ancestor_session_ids: ancestors.iter().map(|id| id.to_string()).collect(),
            pending_bg_task_ids: Default::default(),
            pending_agent_task_ids: Default::default(),
            pending_permission_agent_ids: Default::default(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            engine: Default::default(),
            model: None,
            effort: None,
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

    #[rstest]
    #[case::without_force(false)]
    #[case::with_force(true)]
    fn adopt_command_parses_target_and_force_flag(#[case] force: bool) {
        #[derive(Parser)]
        struct TestCli {
            #[command(subcommand)]
            command: crate::commands::agent::AgentCommands,
        }

        let mut argv = vec!["a", "peer", "adopt", "session-x"];
        if force {
            argv.push("--force");
        }
        let actual = TestCli::try_parse_from(argv)
            .expect("adopt command should parse")
            .command;
        assert!(
            actual
                == crate::commands::agent::AgentCommands::Peer(super::super::PeerCommands::Adopt(
                    AdoptArgs {
                        session_id: "session-x".to_string(),
                        force,
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
    #[case::missing_without_force(None, vec![], false)]
    #[case::missing_with_force(None, vec![], true)]
    #[case::ended_without_force(Some(SessionStatus::Ended), vec![], false)]
    #[case::ended_with_force(Some(SessionStatus::Ended), vec![], true)]
    #[case::ended_direct_child_without_force(
        Some(SessionStatus::Ended),
        vec!["caller"],
        false
    )]
    fn adopt_rejects_missing_or_ended_targets(
        session_store: TestSessionStore,
        #[case] target_status: Option<SessionStatus>,
        #[case] target_ancestors: Vec<&str>,
        #[case] force: bool,
    ) {
        session_store.save(&session("caller", &[], SessionStatus::Running));
        if let Some(status) = target_status {
            session_store.save(&session("target", &target_ancestors, status));
        }

        let error = session_store
            .adopt("caller", "target", force)
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
        store.save(&session("old-parent", &[], SessionStatus::Running));

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
        store.save(&session("old-parent", &[], SessionStatus::Stopped));
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

    #[test]
    fn adopt_rejects_ancestor_reached_through_a_reparented_parent() {
        let store = session_store();
        store.save(&session("caller", &[], SessionStatus::Running));
        store.save(&session("old-parent", &[], SessionStatus::Running));
        store.save(&session("target", &["old-parent"], SessionStatus::Running));
        store.save(&session(
            "descendant",
            &["old-parent", "target"],
            SessionStatus::Running,
        ));
        store
            .adopt("caller", "target", true)
            .expect("force should permit the first reparenting");

        let result = store
            .adopt("descendant", "caller", false)
            .map_err(|error| error.to_string());

        assert_eq!(
            result,
            Err(
                "Cannot adopt ancestor session 'caller' because it would create a cycle"
                    .to_string()
            )
        );
    }

    #[test]
    fn adopt_clears_inherited_vibe_for_the_new_parent() {
        let store = session_store();
        let mut caller = session("caller", &[], SessionStatus::Running);
        caller.vibe_enabled = true;
        store.save(&caller);
        store.save(&session("old-parent", &[], SessionStatus::Running));
        let mut target = session("target", &["old-parent"], SessionStatus::Running);
        target.launched_with_vibe = true;
        store.save(&target);

        store
            .adopt("caller", "target", true)
            .expect("force should permit the adoption");

        let adopted = store.load("target");
        assert_eq!(
            (adopted.ancestor_session_ids, adopted.launched_with_vibe),
            (vec!["caller".to_string()], false)
        );
    }

    #[test]
    fn concurrent_reciprocal_adoptions_do_not_create_a_cycle() {
        use std::sync::{Arc, Barrier};

        let session_store = session_store();
        session_store.save(&session("a", &[], SessionStatus::Running));
        session_store.save(&session("b", &[], SessionStatus::Running));

        let barrier = Arc::new(Barrier::new(3));
        let start_adoption = |caller_id: &'static str, target_id: &'static str| {
            let sessions_dir = session_store.path.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                adopt_in(&sessions_dir, caller_id, target_id, true)
                    .map_err(|error| error.to_string())
            })
        };
        let adopt_b_from_a = start_adoption("a", "b");
        let adopt_a_from_b = start_adoption("b", "a");
        barrier.wait();

        let adoption_results = [
            (
                "a",
                "b",
                adopt_b_from_a
                    .join()
                    .expect("first adoption thread should finish"),
            ),
            (
                "b",
                "a",
                adopt_a_from_b
                    .join()
                    .expect("second adoption thread should finish"),
            ),
        ];
        let session_a = session_store.load("a");
        let session_b = session_store.load("b");
        let parent_id = if session_a.ancestor_session_ids.is_empty() {
            "a"
        } else {
            "b"
        };
        let role = |session_id: &str| {
            if session_id == parent_id {
                "parent"
            } else {
                "child"
            }
        };
        let mut normalized_results = adoption_results
            .iter()
            .map(|(caller_id, target_id, result)| {
                let outcome = match result {
                    Ok(()) => "adopted",
                    Err(error)
                        if error
                            == &format!(
                                "Cannot adopt ancestor session '{target_id}' because it would create a cycle"
                            ) =>
                    {
                        "cycle-rejected"
                    }
                    Err(_) => "other-error",
                };
                (role(caller_id), role(target_id), outcome)
            })
            .collect::<Vec<_>>();
        normalized_results.sort_unstable();

        let mut normalized_sessions = [(&session_a, "a"), (&session_b, "b")]
            .iter()
            .map(|(stored, session_id)| {
                (
                    role(session_id),
                    stored
                        .ancestor_session_ids
                        .iter()
                        .map(|ancestor_id| role(ancestor_id))
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        normalized_sessions.sort_unstable_by_key(|(session_role, _)| *session_role);

        assert_eq!(
            (normalized_results, normalized_sessions),
            (
                vec![
                    ("child", "parent", "cycle-rejected"),
                    ("parent", "child", "adopted"),
                ],
                vec![("child", vec!["parent"]), ("parent", vec![])],
            )
        );
    }
}
