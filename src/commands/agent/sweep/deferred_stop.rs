use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;

use super::super::{store, unreported_stop_notification};

const QUIET_PERIOD: Duration = Duration::from_secs(30 * 60);

pub(super) fn notify_ready(
    sessions_dir: &Path,
    template: Option<&str>,
    dry_run: bool,
) -> Result<usize> {
    let Some(template) = template else {
        return Ok(0);
    };
    notify_ready_at(
        sessions_dir,
        template,
        Utc::now(),
        dry_run,
        |stop, message| {
            unreported_stop_notification::spawn(&stop.session_id, &stop.parent_session_id, message);
        },
    )
}

fn notify_ready_at(
    sessions_dir: &Path,
    template: &str,
    now: chrono::DateTime<Utc>,
    dry_run: bool,
    mut notify: impl FnMut(&store::DeferredStop, &str),
) -> Result<usize> {
    if dry_run {
        return Ok(0);
    }
    let mature = store::take_mature_deferred_stops(sessions_dir, now, QUIET_PERIOD)?;
    let mut notified = 0;
    for stop in mature {
        let message = unreported_stop_notification::render(
            template,
            &stop.session_id,
            stop.last_message.as_deref().unwrap_or_default(),
        );
        tracing::info!(
            event = "agent.sweep.deferred_unreported_stop_notified",
            session = %stop.session_id,
            parent_session = %stop.parent_session_id,
            stopped_at = %stop.stopped_at,
        );
        notify(&stop, &message);
        notified += 1;
    }
    Ok(notified)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use chrono::{TimeZone, Utc};
    use indoc::indoc;
    use rstest::fixture;
    use tempfile::TempDir;

    use super::*;
    use crate::commands::agent::store::{record_deferred_stop, save_session_to};
    use crate::commands::agent::types::{ChildLaunchPermission, Engine, Session, SessionStatus};

    const TEMPLATE: &str = indoc! {"
        <delegation-update>
        Session: {session_id}
        Last message: {last_message}
        </delegation-update>"
    };

    #[fixture]
    fn sessions_dir() -> TempDir {
        TempDir::new().expect("temporary directory should be created")
    }

    fn stopped_delegate() -> Session {
        let timestamp = Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap();
        Session {
            launch_id: None,
            archived_at: None,
            pr_context: None,
            created_pr_urls: Vec::new(),
            session_id: "fixture-delegate".to_string(),
            crit_urls: Vec::new(),
            crit_not_waiting_urls: Default::default(),
            web_uis: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: "/fixture/worktree".into(),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Stopped,
            created_at: timestamp,
            updated_at: timestamp,
            last_prompted_at: None,
            last_message: Some("A fictional progress summary.".to_string()),
            current_tool: None,
            current_tool_name: None,
            label: None,
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            child_launch: ChildLaunchPermission::Deny,
            ancestor_session_ids: vec!["fixture-parent".to_string()],
            pending_bg_task_ids: Default::default(),
            pending_agent_task_ids: Default::default(),
            pending_permission_agent_ids: Default::default(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            agent_status: None,
            agent_status_clear_pending: false,
            engine: Engine::Codex,
            model: None,
            effort: None,
        }
    }

    fn record(session: &Session, sessions_dir: &TempDir, stopped_at: chrono::DateTime<Utc>) {
        save_session_to(sessions_dir.path(), session).unwrap();
        record_deferred_stop(sessions_dir.path(), session, "fixture-parent", stopped_at).unwrap();
    }

    #[rstest::rstest]
    #[case::mature(Utc.with_ymd_and_hms(2026, 10, 1, 9, 30, 0).unwrap(), 1)]
    #[case::too_early(Utc.with_ymd_and_hms(2026, 10, 1, 9, 29, 59).unwrap(), 0)]
    fn sends_a_single_existing_template_notification(
        #[case] now: chrono::DateTime<Utc>,
        #[case] expected_count: usize,
        sessions_dir: TempDir,
    ) {
        let session = stopped_delegate();
        let stopped_at = Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap();
        record(&session, &sessions_dir, stopped_at);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let calls_for_notify = Arc::clone(&calls);

        let notified = notify_ready_at(
            sessions_dir.path(),
            TEMPLATE,
            now,
            false,
            move |stop, message| {
                calls_for_notify
                    .lock()
                    .unwrap()
                    .push((stop.parent_session_id.clone(), message.to_string()))
            },
        )
        .unwrap();
        let next_sweep_count =
            notify_ready_at(sessions_dir.path(), TEMPLATE, now, false, |_, _| {
                panic!("a consumed deferred record must not be notified again")
            })
            .unwrap();

        assert_eq!(
            (notified, next_sweep_count, calls.lock().unwrap().clone()),
            (
                expected_count,
                0,
                if expected_count == 1 {
                    vec![(
                        "fixture-parent".to_string(),
                        indoc! {"
                            <delegation-update>
                            Session: fixture-delegate
                            Last message: A fictional progress summary.
                            </delegation-update>"
                        }
                        .to_string(),
                    )]
                } else {
                    Vec::new()
                },
            ),
        );
    }

    #[rstest::rstest]
    #[case::running(SessionStatus::Running)]
    #[case::waiting_for_input(SessionStatus::WaitingInput)]
    fn drops_a_mature_record_after_the_delegate_resumes(
        #[case] status: SessionStatus,
        sessions_dir: TempDir,
    ) {
        let mut session = stopped_delegate();
        let stopped_at = Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap();
        record(&session, &sessions_dir, stopped_at);
        session.status = status;
        save_session_to(sessions_dir.path(), &session).unwrap();

        let notified = notify_ready_at(
            sessions_dir.path(),
            TEMPLATE,
            stopped_at + chrono::Duration::minutes(31),
            false,
            |_, _| panic!("a resumed delegate must not be notified"),
        )
        .unwrap();

        assert_eq!(notified, 0);
    }

    #[rstest::rstest]
    fn drops_a_stale_record_when_a_later_stop_is_already_saved(sessions_dir: TempDir) {
        let mut session = stopped_delegate();
        let stopped_at = Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap();
        record(&session, &sessions_dir, stopped_at);
        session.updated_at = stopped_at + chrono::Duration::minutes(1);
        save_session_to(sessions_dir.path(), &session).unwrap();

        let notified = notify_ready_at(
            sessions_dir.path(),
            TEMPLATE,
            stopped_at + chrono::Duration::minutes(31),
            false,
            |_, _| panic!("a later Stop must replace an older deferred record"),
        )
        .unwrap();

        assert_eq!(notified, 0);
    }

    #[rstest::rstest]
    fn dry_run_does_not_consume_deferred_records(sessions_dir: TempDir) {
        let session = stopped_delegate();
        let stopped_at = Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap();
        record(&session, &sessions_dir, stopped_at);

        let dry_run_count = notify_ready_at(
            sessions_dir.path(),
            TEMPLATE,
            stopped_at + chrono::Duration::minutes(31),
            true,
            |_, _| panic!("dry run must not send a notification"),
        )
        .unwrap();
        let actual_count = notify_ready_at(
            sessions_dir.path(),
            TEMPLATE,
            stopped_at + chrono::Duration::minutes(31),
            false,
            |_, _| {},
        )
        .unwrap();

        assert_eq!((dry_run_count, actual_count), (0, 1));
    }
}
