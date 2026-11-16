use anyhow::Result;

use super::bg_tasks;
use super::store;
use super::types::{BG_RUN_PENDING_TASK_MARKER, Session};

/// Loads one session with `a agent bg run` registry state included for
/// presentation and background-task decisions.
pub(crate) fn load_session_with_bg_run_status(session_id: &str) -> Result<Option<Session>> {
    let Some(mut session) = store::load_session(session_id)? else {
        return Ok(None);
    };
    include_pending_status(&mut session);
    Ok(Some(session))
}

/// Lists active sessions with `a agent bg run` registry state included for
/// presentation and background-task decisions.
pub(crate) fn list_sessions_with_bg_run_status() -> Result<Vec<Session>> {
    let mut sessions = store::list_sessions()?;
    for session in &mut sessions {
        include_pending_status(session);
    }
    Ok(sessions)
}

/// Includes registry state in the in-memory task set consumed by status and
/// background-task decisions.
pub(crate) fn include_pending_status(session: &mut Session) {
    match bg_tasks::tasks_dir() {
        Ok(root) => include_pending_status_in(session, &root),
        Err(error) => apply_pending_status(session, Err(error)),
    }
}

pub(crate) fn include_pending_status_in(session: &mut Session, tasks_root: &std::path::Path) {
    apply_pending_status(
        session,
        bg_tasks::has_pending_in(tasks_root, &session.session_id),
    );
}

/// Includes registry state on Stop without touching the session record while
/// the hook is preparing its own session update.
pub(crate) fn include_pending_status_for_stop(session: &mut Session) {
    let pending = bg_tasks::tasks_dir().and_then(|root| {
        bg_tasks::scan_pending_in(&root, &session.session_id).map(|scan| scan.pending)
    });
    apply_pending_status(session, pending);
}

fn apply_pending_status(session: &mut Session, pending: Result<bool>) {
    match pending {
        Ok(true) => mark_pending_status(session),
        Ok(false) => {}
        Err(error) => {
            tracing::warn!(
                event = "agent.bg_run.registry_read_failed",
                session = %session.session_id,
                error = %error,
            );
            mark_pending_status(session);
        }
    }
}

fn mark_pending_status(session: &mut Session) {
    session
        .pending_bg_task_ids
        .insert(BG_RUN_PENDING_TASK_MARKER.to_string());
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::TempDir;

    use super::include_pending_status_in;
    use crate::commands::agent::types::{DisplayStatus, Session, SessionStatus};
    use crate::shared::hex;

    #[test]
    fn codex_background_run_registry_marks_session_as_background() {
        let tasks_root = TempDir::new().expect("task registry directory should be created");
        let session_id = "fixture-codex-session";
        let task_id = uuid::Uuid::new_v4().to_string();
        let task_directory = tasks_root.path().join(hex::encode(session_id.as_bytes()));
        std::fs::create_dir_all(&task_directory)
            .expect("task registry session directory should be created");
        std::fs::write(
            task_directory.join(format!("{task_id}.pending")),
            serde_json::to_vec(&json!({
                "parent_pid": std::process::id(),
                "worker_pid": null,
                "command_pid": null,
            }))
            .expect("task marker should serialize"),
        )
        .expect("task marker should be written");
        let mut session: Session = serde_json::from_value(json!({
            "session_id": session_id,
            "cwd": "/fixture/project",
            "engine": "codex",
            "status": "stopped",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
        }))
        .expect("synthetic Codex session should deserialize");

        include_pending_status_in(&mut session, tasks_root.path());

        assert_eq!(
            (session.status, session.display_status()),
            (SessionStatus::Stopped, DisplayStatus::Background),
        );
    }
}
