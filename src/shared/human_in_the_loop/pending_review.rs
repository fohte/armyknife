use std::path::{Path, PathBuf};

use crate::commands::agent::store;
use crate::shared::env_var::EnvVars;

pub(super) struct PendingReviewGuard {
    sessions_dir: PathBuf,
    session_id: String,
    review_id: String,
}

impl PendingReviewGuard {
    pub(super) fn for_current_session(marker: Option<&str>) -> Option<Self> {
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

        match Self::for_session(&sessions_dir, &session_id, marker) {
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

    pub(super) fn for_session(
        sessions_dir: &Path,
        session_id: &str,
        marker: Option<&str>,
    ) -> anyhow::Result<Self> {
        let id = uuid::Uuid::new_v4().to_string();
        let review_id = match marker {
            Some(marker) => format!("{marker}{id}"),
            None => id,
        };
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
            work_type_pinned: false,
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
            agent_status: None,
            engine: Engine::Claude,
        }
    }

    #[test]
    fn review_wait_markers_are_removed_independently() {
        let temp_dir = TempDir::new().expect("temp dir creation should succeed");
        let sessions_dir = temp_dir.path().join("sessions");
        store::save_session_to(&sessions_dir, &session("session-a"))
            .expect("session should be saved");

        let marker = crate::shared::human_in_the_loop::PR_DRAFT_REVIEW_MARKER;
        let (remaining_after_first_finishes, remaining_after_all_finish) = {
            let first = PendingReviewGuard::for_session(&sessions_dir, "session-a", None)
                .expect("first marker should be added");
            let second = PendingReviewGuard::for_session(&sessions_dir, "session-a", Some(marker))
                .expect("second marker should be added");

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

            (remaining_after_first_finishes, remaining_after_all_finish)
        };

        let normalize_ids = |ids: BTreeSet<String>| {
            ids.into_iter()
                .map(|id| {
                    if id.starts_with(marker) {
                        format!("{marker}<generated>")
                    } else {
                        "<generated>".to_string()
                    }
                })
                .collect::<BTreeSet<_>>()
        };

        assert_eq!(
            (
                normalize_ids(remaining_after_first_finishes),
                normalize_ids(remaining_after_all_finish)
            ),
            (
                BTreeSet::from([format!("{marker}<generated>")]),
                BTreeSet::new()
            )
        );
    }
}
