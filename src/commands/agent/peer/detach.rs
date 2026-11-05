use std::path::Path;

use anyhow::Result;
use clap::Args;

use super::{CcError, current_session_id, store};

#[derive(Args, Clone, PartialEq, Eq)]
pub struct DetachArgs {
    /// Session ID of the direct child to detach
    pub session_id: String,
}

pub fn run(args: &DetachArgs) -> Result<()> {
    detach_in(
        &store::sessions_dir()?,
        &current_session_id()?,
        &args.session_id,
    )
}

fn detach_in(sessions_dir: &Path, caller_id: &str, target_id: &str) -> Result<()> {
    let lock = store::lock_session_for_update(sessions_dir, target_id)?;
    let mut target = lock
        .load()?
        .ok_or_else(|| CcError::SessionNotFound(target_id.to_string()))?;
    if !target.is_direct_child_of(caller_id) {
        return Err(CcError::NotDirectChild(target_id.to_string()).into());
    }

    // Keeping any ancestor would make it the new parent, so the whole chain goes.
    target.ancestor_session_ids.clear();
    // Vibe inheritance follows the ancestor chain that was just cleared.
    target.launched_with_vibe = false;
    lock.save(&target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::types::Session;
    use clap::Parser;
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    #[fixture]
    fn sessions_dir() -> TempDir {
        TempDir::new().expect("session directory should be created")
    }

    fn save_session(sessions_dir: &Path, session_id: &str, ancestors: &[&str]) {
        let session: Session = serde_json::from_value(serde_json::json!({
            "session_id": session_id,
            "cwd": "/repo",
            "transcript_path": null,
            "tmux_info": null,
            "status": "running",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_message": null,
            "launched_with_vibe": true,
            "ancestor_session_ids": ancestors,
        }))
        .expect("session should deserialize");
        store::save_session_to(sessions_dir, &session).expect("session should save");
    }

    fn load_session(sessions_dir: &Path, session_id: &str) -> Session {
        store::load_session_from(sessions_dir, session_id)
            .expect("session should load")
            .expect("session should exist")
    }

    #[test]
    fn detach_command_parses_target() {
        #[derive(Parser)]
        struct TestCli {
            #[command(subcommand)]
            command: crate::commands::agent::AgentCommands,
        }

        let actual = TestCli::try_parse_from(["a", "peer", "detach", "session-x"])
            .expect("detach command should parse")
            .command;
        assert!(
            actual
                == crate::commands::agent::AgentCommands::Peer(super::super::PeerCommands::Detach(
                    DetachArgs {
                        session_id: "session-x".to_string(),
                    }
                ))
        );
    }

    #[rstest]
    fn detach_removes_the_target_from_parent_and_children_lookups(sessions_dir: TempDir) {
        let dir = sessions_dir.path();
        save_session(dir, "caller", &["root"]);
        save_session(dir, "target", &["root", "caller"]);
        save_session(dir, "sibling", &["root", "caller"]);

        detach_in(dir, "caller", "target").expect("direct child should detach");

        let sessions = ["caller", "target", "sibling"].map(|id| load_session(dir, id));
        let target = &sessions[1];
        assert_eq!(
            (
                target.ancestor_session_ids.clone(),
                target.launched_with_vibe,
                super::super::filter_parent(&sessions, target)
                    .iter()
                    .map(|session| session.session_id.as_str())
                    .collect::<Vec<_>>(),
                super::super::filter_children(&sessions, "caller")
                    .iter()
                    .map(|session| session.session_id.as_str())
                    .collect::<Vec<_>>(),
            ),
            (Vec::<String>::new(), false, vec![], vec!["sibling"]),
        );
    }

    #[rstest]
    #[case::missing_target(None, "Session not found: target")]
    #[case::no_parent(
        Some(vec![]),
        "Session 'target' is not a direct child of the calling session"
    )]
    #[case::other_parent(
        Some(vec!["other"]),
        "Session 'target' is not a direct child of the calling session"
    )]
    #[case::grandchild(
        Some(vec!["caller", "child"]),
        "Session 'target' is not a direct child of the calling session"
    )]
    fn detach_rejects_targets_that_are_not_direct_children(
        sessions_dir: TempDir,
        #[case] target_ancestors: Option<Vec<&str>>,
        #[case] expected: &str,
    ) {
        let dir = sessions_dir.path();
        if let Some(ancestors) = &target_ancestors {
            save_session(dir, "target", ancestors);
        }

        let result = detach_in(dir, "caller", "target").map_err(|error| error.to_string());

        assert_eq!(result, Err(expected.to_string()));
    }
}
