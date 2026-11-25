use std::io::{self, Write};
use std::time::Duration;

use anyhow::{Result, bail};
use chrono::Utc;

use super::super::bg_tasks::{self, PendingTask};
use super::super::types::{BG_RUN_PENDING_TASK_MARKER, Session, SessionStatus, TmuxInfo};
use super::{CloseRuntime, DraftInspection};

pub(super) fn ensure_session_is_idle(
    session: &Session,
    effective_status: SessionStatus,
) -> Result<()> {
    if session.has_active_work(effective_status) {
        match effective_status {
            SessionStatus::Running | SessionStatus::WaitingInput => bail!(
                "Agent session `{}` is running or waiting for input; complete any requested input and wait for the current turn to finish, then retry this command",
                session.session_id
            ),
            _ => {
                let pending_tasks =
                    bg_tasks::pending_tasks(&session.session_id).unwrap_or_else(|error| {
                        tracing::warn!(
                            event = "agent.close.pending_task_details_failed",
                            session = %session.session_id,
                            error = %error,
                        );
                        Vec::new()
                    });
                bail!("{}", pending_work_error(session, &pending_tasks));
            }
        }
    }
    if session.has_pending_permission_requests() {
        bail!(
            "Agent session `{}` has pending permission requests; resolve them and retry this command",
            session.session_id
        );
    }
    Ok(())
}

fn pending_work_error(session: &Session, pending_tasks: &[PendingTask]) -> String {
    let mut details = vec![format!(
        "Agent session `{}` has pending background or agent tasks:",
        session.session_id
    )];
    let mut reported_background_ids = std::collections::BTreeSet::new();

    for task in pending_tasks {
        let command = task
            .command
            .as_ref()
            .and_then(|arguments| shlex::try_join(arguments.iter().map(String::as_str)).ok())
            .unwrap_or_else(|| task.description.clone());
        details.push(format!(
            "  Background task `{}`: {command} (started at {})",
            task.id,
            task.started_at.format("%Y-%m-%d %H:%M:%S UTC")
        ));
        reported_background_ids.insert(task.id.as_str());
    }

    for task_id in &session.pending_bg_task_ids {
        if task_id != BG_RUN_PENDING_TASK_MARKER
            && !reported_background_ids.contains(task_id.as_str())
        {
            details.push(format!("  Background task ID: `{task_id}`"));
        }
    }
    if pending_tasks.is_empty()
        && session
            .pending_bg_task_ids
            .contains(BG_RUN_PENDING_TASK_MARKER)
    {
        details.push("  Background task details are unavailable.".to_owned());
    }
    for task_id in &session.pending_agent_task_ids {
        details.push(format!("  Agent task ID: `{task_id}`"));
    }
    for review_id in &session.pending_human_review_ids {
        if let Some(marker) = crate::shared::human_in_the_loop::review_document_marker(review_id) {
            details.push(format!(
                "  Review document: {}\n    Finish it in the editor, or run `a agent watch-web` and finish it there if it is the active review document.",
                marker.path.display()
            ));
        }
    }

    details.push("Finish the pending work, then retry this command.".to_owned());
    details.join("\n")
}

pub(super) fn check_draft_state<R: CloseRuntime>(
    runtime: &R,
    tmux_info: &TmuxInfo,
    session: &Session,
) -> Result<DraftInspection> {
    match runtime.has_draft(tmux_info, session.engine) {
        Some(false) => Ok(DraftInspection::NoDraft),
        Some(true) => bail!(
            "Agent session `{}` has an unsent draft; pass --force to close it",
            session.session_id
        ),
        None => check_unknown_draft_state(session, runtime.unknown_draft_timeout()),
    }
}

pub(super) fn check_unknown_draft_state(
    session: &Session,
    timeout: Duration,
) -> Result<DraftInspection> {
    let active = match Utc::now()
        .signed_duration_since(session.updated_at)
        .to_std()
    {
        Ok(elapsed) => elapsed < timeout,
        Err(_) => true,
    };
    tracing::debug!(
        event = "agent.close.draft_state_unavailable",
        session = %session.session_id,
        timeout = ?timeout,
        decision = if active { "blocked_recent_activity" } else { "proceed_after_timeout" },
    );
    if active {
        bail!(
            "Could not check for an unsent draft in agent session `{}`; recent activity prevents closing it, pass --force to close it",
            session.session_id
        );
    }
    Ok(DraftInspection::Unavailable)
}

pub(super) fn validate_session_without_tmux_info(
    session: &Session,
    unknown_draft_timeout: Duration,
) -> Result<DraftInspection> {
    ensure_session_is_idle(session, session.status)?;
    check_unknown_draft_state(session, unknown_draft_timeout)
}

pub(super) fn write_draft_unavailable_warning(
    writer: &mut impl Write,
    session_id: &str,
) -> io::Result<()> {
    writeln!(
        writer,
        "Warning: could not inspect the unsent draft for session `{session_id}`; using its last update time for the close safety check"
    )
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use indoc::indoc;
    use rstest::{fixture, rstest};
    use serde_json::json;

    use super::pending_work_error;
    use crate::commands::agent::bg_tasks::PendingTask;
    use crate::commands::agent::types::{BG_RUN_PENDING_TASK_MARKER, Session};

    #[fixture]
    fn session() -> Session {
        let process_id = std::process::id().to_string();
        let process_started_at = crate::infra::process::process_start_time(std::process::id())
            .expect("test process start time should be available");
        let review_id = [
            "review-id",
            "v3",
            "12345",
            "/fixture/review.md",
            "fixture-queue",
            process_id.as_str(),
            process_started_at.as_str(),
        ]
        .join("\0");
        serde_json::from_value(json!({
            "session_id": "fixture-session",
            "cwd": "/fixture/project",
            "engine": "codex",
            "status": "stopped",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "pending_bg_task_ids": ["bg-task", "hook-task"],
            "pending_agent_task_ids": ["subagent-task"],
            "pending_human_review_ids": [review_id],
        }))
        .expect("fixture session should deserialize")
    }

    fn started_at() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z")
            .expect("fixture timestamp should parse")
            .with_timezone(&Utc)
    }

    #[fixture]
    fn pending_task() -> PendingTask {
        PendingTask {
            id: "bg-task".to_owned(),
            description: "fixture-command".to_owned(),
            command: Some(vec![
                "fixture-command".to_owned(),
                "--target".to_owned(),
                "fixture-review".to_owned(),
            ]),
            started_at: started_at(),
            elapsed_seconds: 0,
            stdout_path: "/fixture/output".into(),
        }
    }

    #[rstest]
    fn pending_work_error_lists_background_command_ids_and_review_document(
        session: Session,
        pending_task: PendingTask,
    ) {
        assert_eq!(
            pending_work_error(&session, &[pending_task]),
            indoc! {"
                Agent session `fixture-session` has pending background or agent tasks:
                  Background task `bg-task`: fixture-command --target fixture-review (started at 2026-01-02 03:04:05 UTC)
                  Background task ID: `hook-task`
                  Agent task ID: `subagent-task`
                  Review document: /fixture/review.md
                    Finish it in the editor, or run `a agent watch-web` and finish it there if it is the active review document.
                Finish the pending work, then retry this command.
            "}.trim_end(),
        );
    }

    #[rstest]
    fn pending_work_error_lists_runtime_marker_task_details_without_exposing_marker(
        mut session: Session,
        pending_task: PendingTask,
    ) {
        session.pending_bg_task_ids.clear();
        session
            .pending_bg_task_ids
            .insert(BG_RUN_PENDING_TASK_MARKER.to_owned());
        session.pending_agent_task_ids.clear();
        session.pending_human_review_ids.clear();

        assert_eq!(
            pending_work_error(&session, &[pending_task]),
            indoc! {"
                Agent session `fixture-session` has pending background or agent tasks:
                  Background task `bg-task`: fixture-command --target fixture-review (started at 2026-01-02 03:04:05 UTC)
                Finish the pending work, then retry this command.
            "}.trim_end(),
        );
    }

    #[rstest]
    fn pending_work_error_uses_legacy_description_when_command_is_missing(mut session: Session) {
        session.pending_bg_task_ids.clear();
        session.pending_agent_task_ids.clear();
        session.pending_human_review_ids.clear();
        let task = PendingTask {
            id: "legacy-task".to_owned(),
            description: "fixture-command".to_owned(),
            command: None,
            started_at: started_at(),
            elapsed_seconds: 0,
            stdout_path: "/fixture/output".into(),
        };

        assert_eq!(
            pending_work_error(&session, &[task]),
            indoc! {"
                Agent session `fixture-session` has pending background or agent tasks:
                  Background task `legacy-task`: fixture-command (started at 2026-01-02 03:04:05 UTC)
                Finish the pending work, then retry this command.
            "}
            .trim_end(),
        );
    }
}
