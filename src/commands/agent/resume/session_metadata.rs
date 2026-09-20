use std::path::Path;

use anyhow::Result;

use crate::commands::agent::store;
use crate::shared::env_var::parse_ancestor_session_ids;

pub(super) fn record_ancestor_session_ids_if_empty(
    sessions_dir: &Path,
    session_id: &str,
    ancestor_session_ids: &str,
) -> Result<()> {
    let ancestor_session_ids = parse_ancestor_session_ids(ancestor_session_ids);
    if ancestor_session_ids.is_empty() {
        return Ok(());
    }

    store::update_session_in(sessions_dir, session_id, |session| {
        if !session.ancestor_session_ids.is_empty() {
            return false;
        }

        session.ancestor_session_ids = ancestor_session_ids;
        true
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use chrono::Utc;
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::*;
    use crate::commands::agent::types::{Engine, Session, SessionStatus};

    #[fixture]
    fn sessions_dir() -> TempDir {
        TempDir::new().expect("temp dir creation should succeed")
    }

    fn session(ancestor_session_ids: Vec<String>) -> Session {
        let now = Utc::now();
        Session {
            session_id: "resume-target".to_string(),
            cwd: PathBuf::from("/workspace/project"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Ended,
            created_at: now,
            updated_at: now,
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids,
            pending_bg_task_ids: BTreeSet::new(),
            pending_agent_task_ids: BTreeSet::new(),
            pending_permission_agent_ids: BTreeSet::new(),
            read_at: None,
            sweep_signaled: false,
            engine: Engine::Codex,
        }
    }

    #[rstest]
    #[case::fills_empty_chain(&[], &["root", "parent"])]
    #[case::preserves_existing_chain(&["current-parent"], &["current-parent"])]
    fn records_only_when_current_chain_is_empty(
        sessions_dir: TempDir,
        #[case] existing_ancestors: &[&str],
        #[case] expected_ancestors: &[&str],
    ) {
        let existing_ancestors = existing_ancestors
            .iter()
            .map(|id| (*id).to_string())
            .collect();
        store::save_session_to(sessions_dir.path(), &session(existing_ancestors))
            .expect("save should succeed");

        record_ancestor_session_ids_if_empty(sessions_dir.path(), "resume-target", "root, parent")
            .expect("record should succeed");
        let actual = store::load_session_from(sessions_dir.path(), "resume-target")
            .expect("load should succeed")
            .expect("session should exist")
            .ancestor_session_ids;

        assert_eq!(
            actual,
            expected_ancestors
                .iter()
                .map(|id| (*id).to_string())
                .collect::<Vec<_>>()
        );
    }
}
