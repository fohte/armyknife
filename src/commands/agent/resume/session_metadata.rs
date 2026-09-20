use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Result;
use chrono::Utc;

use crate::commands::agent::store;
use crate::commands::agent::types::{Engine, Session, SessionStatus};
use crate::shared::env_var::parse_ancestor_session_ids;

pub(super) fn record_ancestor_session_ids_if_empty(
    sessions_dir: &Path,
    session_id: &str,
    cwd: &Path,
    ancestor_session_ids: &str,
) -> Result<()> {
    let ancestor_session_ids = parse_ancestor_session_ids(ancestor_session_ids);
    if ancestor_session_ids.is_empty() {
        return Ok(());
    }

    let session_lock = store::lock_session_for_update(sessions_dir, session_id)?;
    let now = Utc::now();
    let mut session = session_lock.load()?.unwrap_or_else(|| Session {
        session_id: session_id.to_string(),
        cwd: cwd.to_path_buf(),
        transcript_path: None,
        tty: None,
        tmux_info: None,
        status: SessionStatus::Ended,
        created_at: now,
        updated_at: now,
        last_message: None,
        current_tool: None,
        label: None,
        ancestor_session_ids: Vec::new(),
        pending_bg_task_ids: BTreeSet::new(),
        pending_agent_task_ids: BTreeSet::new(),
        pending_permission_agent_ids: BTreeSet::new(),
        read_at: None,
        sweep_signaled: false,
        engine: Engine::Codex,
    });

    if !session.ancestor_session_ids.is_empty() {
        return Ok(());
    }

    session.ancestor_session_ids = ancestor_session_ids;
    session_lock.save(&session)
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

        record_ancestor_session_ids_if_empty(
            sessions_dir.path(),
            "resume-target",
            Path::new("/workspace/project"),
            "root, parent",
        )
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

    #[rstest]
    fn creates_missing_session_with_ancestor_chain(sessions_dir: TempDir) {
        record_ancestor_session_ids_if_empty(
            sessions_dir.path(),
            "missing-session",
            Path::new("/workspace/resume"),
            "root, parent",
        )
        .expect("record should succeed");

        let session = store::load_session_from(sessions_dir.path(), "missing-session")
            .expect("load should succeed")
            .expect("session should exist");
        let mut actual = serde_json::to_value(session).expect("session should serialize");
        actual["created_at"] = serde_json::json!("<timestamp>");
        actual["updated_at"] = serde_json::json!("<timestamp>");

        assert_eq!(
            actual,
            serde_json::json!({
                "session_id": "missing-session",
                "cwd": "/workspace/resume",
                "transcript_path": null,
                "tty": null,
                "tmux_info": null,
                "status": "ended",
                "created_at": "<timestamp>",
                "updated_at": "<timestamp>",
                "last_message": null,
                "current_tool": null,
                "label": null,
                "ancestor_session_ids": ["root", "parent"],
                "pending_bg_task_ids": [],
                "pending_agent_task_ids": [],
                "pending_permission_agent_ids": [],
                "read_at": null,
                "sweep_signaled": false,
                "engine": "codex"
            })
        );
    }
}
