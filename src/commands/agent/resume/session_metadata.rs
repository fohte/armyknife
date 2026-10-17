use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Result;
use chrono::Utc;

use crate::commands::agent::session_pr_context::Capture;
use crate::commands::agent::store;
use crate::commands::agent::types::{Engine, Session, SessionStatus};
use crate::shared::env_var::parse_ancestor_session_ids;

pub(super) fn record_codex_resume_metadata_if_needed(
    sessions_dir: &Path,
    session_id: &str,
    cwd: &Path,
    ancestor_session_ids: Option<&str>,
    launched_with_vibe: bool,
    capture_pr_context: Option<Capture>,
) -> Result<()> {
    let ancestor_session_ids = ancestor_session_ids
        .map(parse_ancestor_session_ids)
        .unwrap_or_default();
    if ancestor_session_ids.is_empty() && !launched_with_vibe {
        return Ok(());
    }

    let session_lock = store::lock_session_for_update(sessions_dir, session_id)?;
    let now = Utc::now();
    let mut session = session_lock.load()?.unwrap_or_else(|| Session {
        archived_at: None,
        pr_context: capture_pr_context.and_then(|capture| capture(cwd)),
        session_id: session_id.to_string(),
        work_type: None,
        work_type_pinned: false,
        vibe_enabled: false,
        launched_with_vibe: false,
        is_delegated: false,
        crit_urls: Vec::new(),
        pending_human_review_ids: Default::default(),
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
        pending_permission_request_ids: Default::default(),
        read_at: None,
        sweep_signaled: false,
        stop_blocked_by_armyknife: false,
        agent_status: None,
        agent_status_clear_pending: false,
        engine: Engine::Codex,
    });

    let mut changed = false;
    if session.ancestor_session_ids.is_empty() && !ancestor_session_ids.is_empty() {
        session.ancestor_session_ids = ancestor_session_ids;
        changed = true;
    }
    if launched_with_vibe && !session.launched_with_vibe {
        session.launched_with_vibe = true;
        changed = true;
    }
    if changed {
        session_lock.save(&session)
    } else {
        Ok(())
    }
}

pub(super) fn mark_vibe_launch_if_session_exists(
    sessions_dir: &Path,
    session_id: &str,
) -> Result<()> {
    store::update_session_in(sessions_dir, session_id, |session| {
        if session.launched_with_vibe {
            return false;
        }
        session.launched_with_vibe = true;
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
    use crate::commands::agent::session_pr_context::fixed_context_capture;
    use crate::commands::agent::types::{Engine, Session, SessionStatus};

    #[fixture]
    fn sessions_dir() -> TempDir {
        TempDir::new().expect("temp dir creation should succeed")
    }

    fn session(ancestor_session_ids: Vec<String>) -> Session {
        let now = Utc::now();
        Session {
            archived_at: None,
            pr_context: None,
            session_id: "resume-target".to_string(),
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            is_delegated: false,
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
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
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            agent_status: None,
            agent_status_clear_pending: false,
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

        record_codex_resume_metadata_if_needed(
            sessions_dir.path(),
            "resume-target",
            Path::new("/workspace/project"),
            Some("root, parent"),
            false,
            None,
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
        record_codex_resume_metadata_if_needed(
            sessions_dir.path(),
            "missing-session",
            Path::new("/workspace/resume"),
            Some("root, parent"),
            false,
            Some(fixed_context_capture),
        )
        .expect("record should succeed");

        let actual_session = store::load_session_from(sessions_dir.path(), "missing-session")
            .expect("load should succeed")
            .expect("session should exist");
        let mut actual = serde_json::to_value(actual_session).expect("session should serialize");
        actual["created_at"] = serde_json::json!("<timestamp>");
        actual["updated_at"] = serde_json::json!("<timestamp>");

        assert_eq!(
            actual,
            serde_json::json!({
                "session_id": "missing-session",
                "pr_context": {
                    "owner": "sample-owner",
                    "repo": "sample-repo",
                    "branch": "sample-branch",
                    "checklist_item_id": "sample-item"
                },
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
                "work_type": null,
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

    #[rstest]
    fn does_not_precreate_missing_claude_session_for_vibe_resume(sessions_dir: TempDir) {
        mark_vibe_launch_if_session_exists(sessions_dir.path(), "missing-session")
            .expect("marker update should succeed");

        let actual = store::load_session_from(sessions_dir.path(), "missing-session")
            .expect("load should succeed")
            .map(|session| serde_json::to_value(session).expect("session should serialize"));

        assert_eq!(actual, None);
    }

    #[rstest]
    #[case::claude(Engine::Claude)]
    #[case::codex(Engine::Codex)]
    fn updates_existing_session_with_vibe_launch_marker(
        sessions_dir: TempDir,
        #[case] engine: Engine,
    ) {
        let mut expected_session = session(Vec::new());
        expected_session.engine = engine;
        store::save_session_to(sessions_dir.path(), &expected_session)
            .expect("existing session should be saved");
        expected_session.launched_with_vibe = true;

        if engine == Engine::Codex {
            record_codex_resume_metadata_if_needed(
                sessions_dir.path(),
                "resume-target",
                Path::new("/workspace/project"),
                None,
                true,
                None,
            )
            .expect("record should succeed");
        } else {
            mark_vibe_launch_if_session_exists(sessions_dir.path(), "resume-target")
                .expect("marker update should succeed");
        }

        let actual = store::load_session_from(sessions_dir.path(), "resume-target")
            .expect("load should succeed")
            .expect("session should exist");

        assert_eq!(
            serde_json::to_value(actual).expect("actual session should serialize"),
            serde_json::to_value(expected_session).expect("expected session should serialize")
        );
    }

    #[rstest]
    fn creates_missing_codex_session_with_vibe_marker_and_ancestors(sessions_dir: TempDir) {
        record_codex_resume_metadata_if_needed(
            sessions_dir.path(),
            "missing-session",
            Path::new("/workspace/resume"),
            Some("root, parent"),
            true,
            Some(fixed_context_capture),
        )
        .expect("record should succeed");

        let actual_session = store::load_session_from(sessions_dir.path(), "missing-session")
            .expect("load should succeed")
            .expect("session should exist");
        let mut expected = session(vec!["root".to_string(), "parent".to_string()]);
        expected.session_id = "missing-session".to_string();
        expected.cwd = PathBuf::from("/workspace/resume");
        expected.pr_context = fixed_context_capture(Path::new("/workspace/resume"));
        expected.launched_with_vibe = true;
        let mut actual = serde_json::to_value(actual_session).expect("session should serialize");
        let mut expected =
            serde_json::to_value(expected).expect("expected session should serialize");
        actual["created_at"] = serde_json::json!("<timestamp>");
        actual["updated_at"] = serde_json::json!("<timestamp>");
        expected["created_at"] = serde_json::json!("<timestamp>");
        expected["updated_at"] = serde_json::json!("<timestamp>");
        assert_eq!(actual, expected);
    }
}
