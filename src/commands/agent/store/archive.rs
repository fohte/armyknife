use std::path::Path;

use anyhow::Result;

use crate::commands::agent::types::{Engine, Session};

pub(crate) fn delete_session_with_archive(session_id: &str) -> Result<()> {
    delete_session_with_archive_in(
        &super::sessions_dir()?,
        session_id,
        crate::commands::agent::codex_steer::archive_thread,
    )
}

fn delete_session_with_archive_in(
    sessions_dir: &Path,
    session_id: &str,
    mut archive_codex_thread: impl FnMut(&str) -> anyhow::Result<()>,
) -> Result<()> {
    if let Some(session) = super::load_session_from(sessions_dir, session_id)? {
        archive_codex_thread_before_delete_with(&session, &mut archive_codex_thread);
    }

    super::delete_session_from(sessions_dir, session_id)
}

pub(super) fn archive_codex_thread_before_delete_with(
    session: &Session,
    archive_codex_thread: &mut impl FnMut(&str) -> anyhow::Result<()>,
) {
    if session.engine != Engine::Codex {
        return;
    }

    if let Err(error) = archive_codex_thread(&session.session_id) {
        eprintln!(
            "Warning: Failed to archive Codex thread for session {}: {error:#}",
            session.session_id
        );
        tracing::warn!(
            target: "armyknife::commands::agent::store",
            event = "store.codex_archive.err",
            session = %session.session_id,
            msg = format!("failed to archive Codex thread: {error:#}"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::types::{SessionStatus, TmuxInfo};
    use chrono::Utc;
    use rstest::{fixture, rstest};
    use std::cell::RefCell;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    struct TestSessionDir {
        #[expect(dead_code, reason = "kept alive to prevent cleanup until dropped")]
        temp_dir: TempDir,
        sessions_dir: PathBuf,
    }

    #[fixture]
    fn test_session_dir() -> TestSessionDir {
        let temp_dir = TempDir::new().expect("temp directory creation should succeed");
        let sessions_dir = temp_dir.path().join("sessions");
        fs::create_dir_all(&sessions_dir).expect("sessions directory creation should succeed");
        TestSessionDir {
            temp_dir,
            sessions_dir,
        }
    }

    fn make_session(session_id: &str, engine: Engine) -> Session {
        Session {
            session_id: session_id.to_string(),
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: PathBuf::from("/tmp/example-worktree"),
            transcript_path: None,
            tty: None,
            tmux_info: Some(TmuxInfo {
                session_name: "example".to_string(),
                window_name: "example".to_string(),
                window_index: 0,
                pane_id: "%1".to_string(),
            }),
            status: SessionStatus::Stopped,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
            label: None,
            work_type: None,
            work_type_pinned: false,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: Default::default(),
            pending_agent_task_ids: Default::default(),
            pending_permission_agent_ids: Default::default(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            engine,
        }
    }

    #[rstest]
    #[case::claude(Engine::Claude, Vec::new())]
    #[case::codex(Engine::Codex, vec![("session-to-delete".to_string(), true)])]
    fn archives_codex_before_deleting_session(
        test_session_dir: TestSessionDir,
        #[case] engine: Engine,
        #[case] expected_archive_calls: Vec<(String, bool)>,
    ) {
        let session = make_session("session-to-delete", engine);
        super::super::save_session_to(&test_session_dir.sessions_dir, &session)
            .expect("session save should succeed");
        let session_path = test_session_dir.sessions_dir.join("session-to-delete.json");
        let archive_calls = RefCell::new(Vec::new());

        delete_session_with_archive_in(
            &test_session_dir.sessions_dir,
            &session.session_id,
            |session_id| {
                archive_calls
                    .borrow_mut()
                    .push((session_id.to_string(), session_path.exists()));
                Ok(())
            },
        )
        .expect("session deletion should succeed");

        assert_eq!(
            (archive_calls.into_inner(), session_path.exists()),
            (expected_archive_calls, false),
        );
    }

    #[rstest]
    fn continues_deleting_when_archive_fails(test_session_dir: TestSessionDir) {
        let session = make_session("session-to-delete", Engine::Codex);
        super::super::save_session_to(&test_session_dir.sessions_dir, &session)
            .expect("session save should succeed");
        let session_path = test_session_dir.sessions_dir.join("session-to-delete.json");
        let archive_calls = RefCell::new(Vec::new());

        let result = delete_session_with_archive_in(
            &test_session_dir.sessions_dir,
            &session.session_id,
            |session_id| {
                archive_calls
                    .borrow_mut()
                    .push((session_id.to_string(), session_path.exists()));
                Err(anyhow::anyhow!("fixture archive failure"))
            },
        );

        assert_eq!(
            (
                result.is_ok(),
                archive_calls.into_inner(),
                session_path.exists(),
            ),
            (true, vec![("session-to-delete".to_string(), true)], false,),
        );
    }
}
