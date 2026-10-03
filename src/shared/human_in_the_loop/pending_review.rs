use std::path::{Path, PathBuf};

use crate::commands::agent::store;
use crate::shared::env_var::EnvVars;

pub(super) struct PendingReviewGuard {
    sessions_dir: PathBuf,
    session_id: String,
    review_id: String,
}

impl PendingReviewGuard {
    pub(super) fn for_current_session() -> Option<Self> {
        let session_id = EnvVars::load().own_session_id()?;
        let sessions_dir = match store::sessions_dir() {
            Ok(path) => path,
            Err(error) => {
                tracing::warn!(
                    event = "hitl.pending_review.mark_failed",
                    error = %error,
                );
                return None;
            }
        };

        match Self::for_session(&sessions_dir, &session_id) {
            Ok(guard) => Some(guard),
            Err(error) => {
                tracing::warn!(
                    event = "hitl.pending_review.mark_failed",
                    error = %error,
                );
                None
            }
        }
    }

    pub(super) fn for_session(sessions_dir: &Path, session_id: &str) -> anyhow::Result<Self> {
        let review_id = uuid::Uuid::new_v4().to_string();
        store::update_session_in(sessions_dir, session_id, |session| {
            session.pending_human_review_ids.insert(review_id.clone());
            true
        })?;

        Ok(Self {
            sessions_dir: sessions_dir.to_path_buf(),
            session_id: session_id.to_string(),
            review_id,
        })
    }
}

impl Drop for PendingReviewGuard {
    fn drop(&mut self) {
        if let Err(error) =
            store::update_session_in(&self.sessions_dir, &self.session_id, |session| {
                session.pending_human_review_ids.remove(&self.review_id)
            })
        {
            tracing::warn!(
                event = "hitl.pending_review.clear_failed",
                session = %self.session_id,
                error = %error,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use chrono::Utc;
    use tempfile::TempDir;

    use crate::commands::agent::store;
    use crate::commands::agent::types::{Engine, Session, SessionStatus};

    use super::PendingReviewGuard;

    fn session(session_id: &str) -> Session {
        Session {
            session_id: session_id.to_string(),
            work_type: None,
            crit_urls: Vec::new(),
            pending_human_review_ids: BTreeSet::new(),
            cwd: PathBuf::from("/tmp/test"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Stopped,
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
            engine: Engine::Claude,
        }
    }

    #[test]
    fn review_wait_markers_are_removed_independently() {
        let temp_dir = TempDir::new().expect("temp dir creation should succeed");
        let sessions_dir = temp_dir.path().join("sessions");
        store::save_session_to(&sessions_dir, &session("session-a"))
            .expect("session should be saved");

        let ((remaining_after_first_finishes, remaining_after_all_finish), second_id) = {
            let first = PendingReviewGuard::for_session(&sessions_dir, "session-a")
                .expect("first marker should be added");
            let second = PendingReviewGuard::for_session(&sessions_dir, "session-a")
                .expect("second marker should be added");
            let second_id = second.review_id.clone();

            drop(first);
            let remaining_after_first_finishes =
                store::load_session_from(&sessions_dir, "session-a")
                    .expect("session should load")
                    .expect("session should exist")
                    .pending_human_review_ids;

            drop(second);
            let remaining_after_all_finish = store::load_session_from(&sessions_dir, "session-a")
                .expect("session should load")
                .expect("session should exist")
                .pending_human_review_ids;

            (
                (remaining_after_first_finishes, remaining_after_all_finish),
                second_id,
            )
        };

        assert_eq!(
            (remaining_after_first_finishes, remaining_after_all_finish),
            (BTreeSet::from([second_id]), BTreeSet::new())
        );
    }
}
