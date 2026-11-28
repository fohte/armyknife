use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::commands::agent::store;
use crate::shared::env_var::EnvVars;
use crate::shared::human_in_the_loop::LockGuard;

use super::PR_DRAFT_REVIEW_MARKER;

pub(super) struct PendingReviewGuard {
    sessions_dir: PathBuf,
    session_id: String,
    review_id: String,
}

impl PendingReviewGuard {
    pub(super) fn for_current_session(
        marker: Option<&str>,
        review_path: &Path,
        queue_key: Option<&str>,
    ) -> Option<Self> {
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

        match Self::for_session_with_path(
            &sessions_dir,
            &session_id,
            marker,
            review_path,
            queue_key,
        ) {
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

    #[cfg(test)]
    pub(super) fn for_session(
        sessions_dir: &Path,
        session_id: &str,
        marker: Option<&str>,
    ) -> anyhow::Result<Self> {
        Self::for_session_inner(sessions_dir, session_id, marker, None)
    }

    pub(super) fn for_session_with_path(
        sessions_dir: &Path,
        session_id: &str,
        marker: Option<&str>,
        review_path: &Path,
        queue_key: Option<&str>,
    ) -> anyhow::Result<Self> {
        Self::for_session_inner(
            sessions_dir,
            session_id,
            marker,
            Some((review_path, queue_key)),
        )
    }

    fn for_session_inner(
        sessions_dir: &Path,
        session_id: &str,
        marker: Option<&str>,
        review: Option<(&Path, Option<&str>)>,
    ) -> anyhow::Result<Self> {
        let id = uuid::Uuid::new_v4().to_string();
        let marker_id = match marker {
            Some(marker) => format!("{marker}{id}"),
            None => id,
        };
        let review_id = match review {
            Some((path, queue_key)) => {
                let started_at = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                let path = path.to_string_lossy();
                let queue_key = queue_key.unwrap_or_default();
                match crate::infra::process::process_start_time(std::process::id()) {
                    Some(process_started_at) => format!(
                        "{marker_id}\0v3\0{started_at}\0{path}\0{queue_key}\0{}\0{process_started_at}",
                        std::process::id(),
                    ),
                    None => format!("{marker_id}\0v2\0{started_at}\0{path}\0{queue_key}"),
                }
            }
            None => marker_id,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReviewDocumentMarker {
    pub(crate) id: String,
    pub(crate) path: PathBuf,
    pub(crate) queue_key: Option<String>,
    pub(crate) started_at: u128,
}

/// Returns document metadata for a recognized review marker, including legacy formats.
pub(crate) fn review_document_marker(review_id: &str) -> Option<ReviewDocumentMarker> {
    review_document_marker_with(review_id, crate::infra::process::process_start_time_matches)
}

fn review_document_marker_with(
    review_id: &str,
    process_start_time_matches: impl Fn(u32, &str) -> bool,
) -> Option<ReviewDocumentMarker> {
    let (id, metadata) = review_id.split_once('\0')?;
    let mut fields = metadata.split('\0');
    let first = fields.next()?;
    let (started_at, path, queue_key) = if first == "v3" {
        let started_at = fields.next().and_then(|value| value.parse::<u128>().ok())?;
        let path = fields.next()?;
        let queue_key = fields.next()?;
        let pid = fields.next().and_then(|value| value.parse::<u32>().ok())?;
        let process_started_at = fields.next()?;
        if !process_start_time_matches(pid, process_started_at)
            && !LockGuard::is_locked(Path::new(path))
        {
            return None;
        }
        (started_at, path, Some(queue_key))
    } else if first == "v2" {
        let started_at = fields.next().and_then(|value| value.parse::<u128>().ok())?;
        let path = fields.next()?;
        (started_at, path, fields.next())
    } else {
        (0, first, None)
    };
    if path.is_empty() || (first != "v3" && !LockGuard::is_locked(Path::new(path))) {
        return None;
    }
    Some(ReviewDocumentMarker {
        id: id.to_string(),
        path: PathBuf::from(path),
        queue_key: queue_key.filter(|key| !key.is_empty()).map(str::to_string),
        started_at,
    })
}

pub(crate) fn is_active_pr_draft_review_marker(review_id: &str) -> bool {
    review_id.starts_with(PR_DRAFT_REVIEW_MARKER)
        && if review_id.contains('\0') {
            review_document_marker(review_id).is_some()
        } else {
            true
        }
}

/// Returns the document path stored in a recognized review marker.
pub(crate) fn review_document_path(review_id: &str) -> Option<PathBuf> {
    review_document_marker(review_id).map(|marker| marker.path)
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
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use crate::commands::agent::store;
    use crate::commands::agent::types::{Engine, Session, SessionStatus};
    use crate::shared::human_in_the_loop::{LockGuard, ReviewDocumentOperationLock};

    use super::{
        PendingReviewGuard, ReviewDocumentMarker, review_document_marker,
        review_document_marker_with,
    };

    fn session(session_id: &str) -> Session {
        Session {
            launch_id: None,
            archived_at: None,
            pr_context: None,
            created_pr_urls: Vec::new(),
            session_id: session_id.to_string(),
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            child_launch: Default::default(),
            crit_urls: Vec::new(),
            crit_not_waiting_urls: Default::default(),
            web_uis: Vec::new(),
            pending_human_review_ids: BTreeSet::new(),
            cwd: PathBuf::from("/tmp/test"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Stopped,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_prompted_at: None,
            last_message: None,
            current_tool: None,
            current_tool_name: None,
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
            engine: Engine::Claude,
            model: None,
            effort: None,
        }
    }

    struct ReviewMarkerFixture {
        _temp_dir: TempDir,
        draft_path: PathBuf,
        marker: String,
        expected_marker: super::ReviewDocumentMarker,
    }

    #[fixture]
    fn review_marker_fixture() -> ReviewMarkerFixture {
        let temp_dir = TempDir::new().expect("temp dir creation should succeed");
        let draft_path = temp_dir.path().join("fixture-draft.md");
        let marker = [
            "review-id",
            "v3",
            "123",
            draft_path.to_str().expect("fixture path should be UTF-8"),
            "queue",
            "41",
            "linux:987",
        ]
        .join("\0");

        ReviewMarkerFixture {
            _temp_dir: temp_dir,
            draft_path: draft_path.clone(),
            marker,
            expected_marker: super::ReviewDocumentMarker {
                id: "review-id".to_string(),
                path: draft_path,
                queue_key: Some("queue".to_string()),
                started_at: 123,
            },
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

    #[test]
    fn review_document_metadata_is_stored_and_removed_with_its_marker() {
        let temp_dir = TempDir::new().expect("temp dir creation should succeed");
        let sessions_dir = temp_dir.path().join("sessions");
        let draft_path = temp_dir.path().join("fixture-draft.md");
        store::save_session_to(&sessions_dir, &session("session-a"))
            .expect("session should be saved");

        let guard = PendingReviewGuard::for_session_with_path(
            &sessions_dir,
            "session-a",
            None,
            &draft_path,
            Some("fixture-review-session"),
        )
        .expect("review document marker should be added");
        let stored_markers = store::load_session_from(&sessions_dir, "session-a")
            .expect("session should load")
            .expect("session should exist")
            .pending_human_review_ids;
        let stored_marker = stored_markers
            .iter()
            .find_map(|review_id| review_document_marker(review_id));

        drop(guard);
        let remaining_markers = store::load_session_from(&sessions_dir, "session-a")
            .expect("session should load")
            .expect("session should exist")
            .pending_human_review_ids;

        assert_eq!(
            (
                stored_marker.map(|marker| {
                    (
                        marker.id.is_empty(),
                        marker.path,
                        marker.queue_key,
                        marker.started_at > 0,
                    )
                }),
                remaining_markers,
            ),
            (
                Some((
                    false,
                    draft_path,
                    Some("fixture-review-session".to_string()),
                    true,
                )),
                BTreeSet::new()
            )
        );
    }

    #[test]
    fn review_document_marker_is_ignored_when_the_owner_process_has_exited() {
        let marker = [
            "review-id",
            "v3",
            "123",
            "/fixture/draft.md",
            "queue",
            "41",
            "linux:987",
        ]
        .join("\0");

        assert_eq!(review_document_marker_with(&marker, |_, _| false), None);
    }

    #[rstest]
    fn review_document_marker_stays_active_while_an_editor_holds_the_lock(
        review_marker_fixture: ReviewMarkerFixture,
    ) {
        let lock = LockGuard::acquire(&review_marker_fixture.draft_path)
            .expect("editor review lock should be active");

        assert_eq!(
            review_document_marker_with(&review_marker_fixture.marker, |_, _| false),
            Some(review_marker_fixture.expected_marker.clone())
        );
        drop(lock);
        assert_eq!(
            review_document_marker_with(&review_marker_fixture.marker, |_, _| false),
            None
        );
    }

    #[rstest]
    fn review_document_marker_does_not_block_while_document_operation_lock_is_held(
        review_marker_fixture: ReviewMarkerFixture,
    ) {
        use std::sync::mpsc;
        use std::time::Duration;

        let _review_guard = LockGuard::acquire(&review_marker_fixture.draft_path)
            .expect("review lock should be active");
        let operation_lock =
            ReviewDocumentOperationLock::acquire(&review_marker_fixture.draft_path)
                .expect("review document operation lock should be acquired");
        let (started_sender, started_receiver) = mpsc::channel();
        let (result_sender, result_receiver) = mpsc::channel();
        let marker = review_marker_fixture.marker;
        let expected_marker = Some(review_marker_fixture.expected_marker);
        let worker = std::thread::spawn(move || {
            started_sender.send(()).expect("worker should report start");
            let result = review_document_marker_with(&marker, |_, _| false);
            result_sender
                .send(result)
                .expect("worker should send marker result");
        });

        started_receiver
            .recv()
            .expect("worker should report start before checking the marker");
        let immediate_result = result_receiver.recv_timeout(Duration::from_secs(1));
        let completed_while_locked = immediate_result.is_ok();
        drop(operation_lock);
        let final_result = match immediate_result {
            Ok(result) => Some(result),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                result_receiver.recv_timeout(Duration::from_secs(1)).ok()
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => None,
        }
        .flatten();
        worker.join().expect("marker check worker should finish");

        assert_eq!(
            (completed_while_locked, final_result),
            (true, expected_marker)
        );
    }

    #[rstest]
    #[case::v2_without_lock("v2", false)]
    #[case::v2_with_lock("v2", true)]
    #[case::legacy_without_lock("legacy", false)]
    #[case::legacy_with_lock("legacy", true)]
    fn older_review_document_markers_require_a_lock(
        #[case] format: &str,
        #[case] lock_active: bool,
        review_marker_fixture: ReviewMarkerFixture,
    ) {
        let _lock = lock_active.then(|| {
            LockGuard::acquire(&review_marker_fixture.draft_path)
                .expect("review lock should be active")
        });
        let review_id = match format {
            "v2" => [
                "legacy-id".to_string(),
                "v2".to_string(),
                "123".to_string(),
                review_marker_fixture.draft_path.display().to_string(),
                "queue".to_string(),
            ]
            .join("\0"),
            "legacy" => format!("legacy-id\0{}", review_marker_fixture.draft_path.display()),
            _ => unreachable!("test case uses a known marker format"),
        };
        let expected = lock_active.then(|| ReviewDocumentMarker {
            id: "legacy-id".to_string(),
            path: review_marker_fixture.draft_path,
            queue_key: (format == "v2").then(|| "queue".to_string()),
            started_at: if format == "v2" { 123 } else { 0 },
        });

        assert_eq!(review_document_marker(&review_id), expected);
    }
}
