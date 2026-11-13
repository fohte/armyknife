use std::path::Path;

use anyhow::Result;
use chrono::Utc;

use super::super::{
    store,
    types::{Engine, Session, SessionStatus},
};
use super::side_effects::SideEffects;
use crate::shared::config;

pub(super) fn codex_process_exited(session_id: &str) -> Result<()> {
    let sessions_dir = store::sessions_dir()?;
    codex_process_exited_in(session_id, &sessions_dir, &SideEffects::all())
}

pub(super) fn end_codex_session(session_id: &str) -> Result<()> {
    let sessions_dir = store::sessions_dir()?;
    end_codex_session_in(session_id, &sessions_dir, &SideEffects::all())
}

pub(super) fn mark_session_ended_after_close(session_id: &str) -> Result<()> {
    let sessions_dir = store::sessions_dir()?;
    mark_session_ended_after_close_in(session_id, &sessions_dir, &SideEffects::all())
}

pub(super) fn mark_session_ended_after_close_in(
    session_id: &str,
    sessions_dir: &Path,
    side_effects: &SideEffects,
) -> Result<()> {
    let Some(current_session) = store::load_session_from(sessions_dir, session_id)? else {
        return Ok(());
    };
    if current_session.engine == Engine::Claude {
        store::set_watch_web_compacting(sessions_dir, session_id, false)?;
    }

    let mut ended_session = None;
    store::update_session_in(sessions_dir, session_id, |session| {
        if session.status == SessionStatus::Ended {
            return false;
        }

        session.status = SessionStatus::Ended;
        session.sweep_signaled = false;
        session.updated_at = Utc::now();
        ended_session = Some(session.clone());
        true
    })?;

    if let Some(session) = ended_session {
        finish_ended_session(session_id, &session, sessions_dir, side_effects);
    }

    Ok(())
}

fn finish_ended_session(
    session_id: &str,
    session: &Session,
    sessions_dir: &Path,
    side_effects: &SideEffects,
) {
    side_effects.remove_notification_group(session_id);
    side_effects.archive_tq_session(session_id);
    let work_type_config = session
        .work_type
        .as_ref()
        .map(|_| config::load_config_or_default());
    side_effects.sync_tmux(
        session.tmux_info.as_ref().map(|info| info.pane_id.as_str()),
        Some(SessionStatus::Ended),
        Some(session),
        work_type_config.as_ref().map(|config| &config.agent),
        sessions_dir,
    );
}

pub(super) fn codex_process_exited_in(
    session_id: &str,
    sessions_dir: &Path,
    side_effects: &SideEffects,
) -> Result<()> {
    end_codex_session_with_policy(session_id, sessions_dir, side_effects, true)
}

pub(super) fn codex_session_end_hook_in(
    session_id: &str,
    sessions_dir: &Path,
    side_effects: &SideEffects,
) -> Result<()> {
    let mut session_for_sync = None;
    store::update_session_in(sessions_dir, session_id, |session| {
        if session.engine != Engine::Codex {
            return false;
        }

        if session.status == SessionStatus::Ended {
            session_for_sync = Some(session.clone());
            return false;
        }

        match preserve_sweep_pause(session) {
            SweepPauseOutcome::AlreadyPaused => {
                session_for_sync = Some(session.clone());
                return false;
            }
            SweepPauseOutcome::Confirmed => {
                session.restore_from_archive();
                session_for_sync = Some(session.clone());
                return true;
            }
            SweepPauseOutcome::NotRequested => {}
        }

        session.status = SessionStatus::Stopped;
        session.updated_at = Utc::now();
        session.restore_from_archive();
        session_for_sync = Some(session.clone());
        true
    })?;

    if let Some(session) = session_for_sync {
        let work_type_config = session
            .work_type
            .as_ref()
            .map(|_| config::load_config_or_default());
        side_effects.sync_tmux(
            session.tmux_info.as_ref().map(|info| info.pane_id.as_str()),
            Some(session.status),
            Some(&session),
            work_type_config.as_ref().map(|config| &config.agent),
            sessions_dir,
        );
    }

    Ok(())
}

pub(super) enum SweepPauseOutcome {
    AlreadyPaused,
    Confirmed,
    NotRequested,
}

pub(super) fn preserve_sweep_pause(
    session: &mut super::super::types::Session,
) -> SweepPauseOutcome {
    if session.status == SessionStatus::Paused {
        return SweepPauseOutcome::AlreadyPaused;
    }
    if !session.sweep_signaled {
        return SweepPauseOutcome::NotRequested;
    }
    session.status = SessionStatus::Paused;
    session.sweep_signaled = false;
    SweepPauseOutcome::Confirmed
}

pub(super) fn end_codex_session_in(
    session_id: &str,
    sessions_dir: &Path,
    side_effects: &SideEffects,
) -> Result<()> {
    end_codex_session_with_policy(session_id, sessions_dir, side_effects, false)
}

fn end_codex_session_with_policy(
    session_id: &str,
    sessions_dir: &Path,
    side_effects: &SideEffects,
    preserve_sweep_pause: bool,
) -> Result<()> {
    let mut ended_session = None;
    store::update_session_in(sessions_dir, session_id, |session| {
        if session.engine != Engine::Codex
            || session.status == SessionStatus::Ended
            || (preserve_sweep_pause
                && (session.status == SessionStatus::Paused || session.sweep_signaled))
        {
            return false;
        }

        session.status = SessionStatus::Ended;
        session.sweep_signaled = false;
        session.updated_at = Utc::now();
        session.restore_from_archive();
        ended_session = Some(session.clone());
        true
    })?;

    if let Some(session) = ended_session {
        finish_ended_session(session_id, &session, sessions_dir, side_effects);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::{Arc, Mutex};

    use chrono::Utc;
    use rstest::{fixture, rstest};
    use rusqlite::OptionalExtension;
    use tempfile::TempDir;

    use super::*;
    use crate::commands::agent::types::{Engine, Session, SessionStatus, TmuxInfo};

    type RecordedIds = Arc<Mutex<Vec<String>>>;
    type RecordedSyncs = Arc<
        Mutex<
            Vec<(
                Option<String>,
                Option<SessionStatus>,
                Option<String>,
                std::path::PathBuf,
            )>,
        >,
    >;

    struct RecordingSideEffects {
        side_effects: SideEffects,
        removed: RecordedIds,
        archived: RecordedIds,
        synced: RecordedSyncs,
    }

    struct LifecycleTestContext {
        temp_dir: TempDir,
        sinks: RecordingSideEffects,
    }

    #[fixture]
    fn lifecycle_test_context() -> LifecycleTestContext {
        LifecycleTestContext {
            temp_dir: TempDir::new().expect("temp dir"),
            sinks: recording_side_effects(),
        }
    }

    fn session(engine: Engine, status: SessionStatus, sweep_signaled: bool) -> Session {
        let now = Utc::now();
        Session {
            archived_at: None,
            pr_context: None,
            session_id: "thread-one".to_string(),
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            child_launch: Default::default(),
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: "/tmp/test".into(),
            transcript_path: None,
            tty: None,
            tmux_info: Some(TmuxInfo {
                session_name: "main".to_string(),
                window_name: "agent".to_string(),
                window_index: 0,
                pane_id: "%42".to_string(),
                socket_path: None,
            }),
            status,
            created_at: now,
            updated_at: now,
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
            sweep_signaled,
            stop_blocked_by_armyknife: false,
            agent_status: None,
            agent_status_clear_pending: false,
            engine,
            model: None,
            effort: None,
        }
    }

    fn recording_side_effects() -> RecordingSideEffects {
        let removed = Arc::new(Mutex::new(Vec::new()));
        let archived = Arc::new(Mutex::new(Vec::new()));
        let synced = Arc::new(Mutex::new(Vec::new()));
        let mut side_effects = SideEffects::none();
        side_effects.removed_notification_groups = Some(removed.clone());
        side_effects.tq_archive = true;
        side_effects.tq_archive_calls = Some(archived.clone());
        side_effects.tmux_sync_calls = Some(synced.clone());
        RecordingSideEffects {
            side_effects,
            removed,
            archived,
            synced,
        }
    }

    fn compacting_state(sessions_dir: &Path, session_id: &str) -> Option<bool> {
        rusqlite::Connection::open(sessions_dir.join("sessions.db"))
            .expect("open session database")
            .query_row(
                "SELECT is_compacting FROM watch_web_session_state WHERE session_id = ?1",
                [session_id],
                |row| row.get::<_, Option<bool>>(0),
            )
            .optional()
            .expect("load compacting state")
            .flatten()
    }

    #[rstest]
    #[case::codex_stopped(Engine::Codex, SessionStatus::Stopped, false, SessionStatus::Ended, vec!["thread-one"], vec!["thread-one"])]
    #[case::codex_paused(Engine::Codex, SessionStatus::Paused, false, SessionStatus::Paused, Vec::<&str>::new(), Vec::<&str>::new())]
    #[case::codex_sweep_signaled(Engine::Codex, SessionStatus::Stopped, true, SessionStatus::Stopped, Vec::<&str>::new(), Vec::<&str>::new())]
    #[case::claude(Engine::Claude, SessionStatus::Stopped, false, SessionStatus::Stopped, Vec::<&str>::new(), Vec::<&str>::new())]
    fn process_exit_ends_only_confirmed_codex_sessions(
        lifecycle_test_context: LifecycleTestContext,
        #[case] engine: Engine,
        #[case] initial_status: SessionStatus,
        #[case] sweep_signaled: bool,
        #[case] expected_status: SessionStatus,
        #[case] expected_removed: Vec<&str>,
        #[case] expected_archived: Vec<&str>,
    ) {
        let LifecycleTestContext { temp_dir, sinks } = lifecycle_test_context;
        store::save_session_to(
            temp_dir.path(),
            &session(engine, initial_status, sweep_signaled),
        )
        .expect("save session");

        codex_process_exited_in("thread-one", temp_dir.path(), &sinks.side_effects)
            .expect("process exit should be recorded");

        let updated = store::load_session_from(temp_dir.path(), "thread-one")
            .expect("load session")
            .expect("session exists");
        let actual = (
            updated.status,
            sinks.removed.lock().expect("removed lock").clone(),
            sinks.archived.lock().expect("archive lock").clone(),
            sinks.synced.lock().expect("sync lock").clone(),
            updated.archived_at,
        );
        let expected_sync = if expected_status == SessionStatus::Ended {
            vec![(
                Some("%42".to_string()),
                Some(SessionStatus::Ended),
                Some("thread-one".to_string()),
                temp_dir.path().to_path_buf(),
            )]
        } else {
            Vec::new()
        };
        assert_eq!(
            actual,
            (
                expected_status,
                expected_removed.into_iter().map(str::to_string).collect(),
                expected_archived.into_iter().map(str::to_string).collect(),
                expected_sync,
                None,
            ),
        );
    }

    #[rstest]
    fn process_exit_restores_archived_codex_session(lifecycle_test_context: LifecycleTestContext) {
        let LifecycleTestContext { temp_dir, sinks } = lifecycle_test_context;
        let mut archived = session(Engine::Codex, SessionStatus::Stopped, false);
        archived.archived_at = Some(Utc::now());
        store::save_session_to(temp_dir.path(), &archived).expect("save session");

        codex_process_exited_in("thread-one", temp_dir.path(), &sinks.side_effects)
            .expect("process exit should be recorded");

        let updated = store::load_session_from(temp_dir.path(), "thread-one")
            .expect("load session")
            .expect("session exists");
        assert_eq!(
            (updated.status, updated.archived_at),
            (SessionStatus::Ended, None)
        );
    }

    #[rstest]
    fn explicit_close_ends_a_paused_codex_session(lifecycle_test_context: LifecycleTestContext) {
        let LifecycleTestContext { temp_dir, sinks } = lifecycle_test_context;
        store::save_session_to(
            temp_dir.path(),
            &session(Engine::Codex, SessionStatus::Paused, false),
        )
        .expect("save session");

        end_codex_session_in("thread-one", temp_dir.path(), &sinks.side_effects)
            .expect("explicit close should confirm the session end");

        let updated = store::load_session_from(temp_dir.path(), "thread-one")
            .expect("load session")
            .expect("session exists");
        assert_eq!(
            (
                updated.status,
                sinks.removed.lock().expect("removed lock").clone(),
                sinks.archived.lock().expect("archive lock").clone(),
            ),
            (
                SessionStatus::Ended,
                vec!["thread-one".to_string()],
                vec!["thread-one".to_string()],
            ),
        );
    }

    #[rstest]
    #[case::claude_paused(Engine::Claude)]
    #[case::codex_paused(Engine::Codex)]
    fn explicit_close_ends_paused_sessions_without_archiving_them(
        lifecycle_test_context: LifecycleTestContext,
        #[case] engine: Engine,
    ) {
        let LifecycleTestContext { temp_dir, sinks } = lifecycle_test_context;
        let mut paused = session(engine, SessionStatus::Paused, true);
        paused.ancestor_session_ids = vec!["fixture-parent-session".to_string()];
        store::save_session_to(temp_dir.path(), &paused).expect("save session");
        if engine == Engine::Claude {
            store::set_watch_web_compacting(temp_dir.path(), "thread-one", true)
                .expect("compacting state should be set");
        }

        mark_session_ended_after_close_in("thread-one", temp_dir.path(), &sinks.side_effects)
            .expect("explicit close should end the session");

        let updated = store::load_session_from(temp_dir.path(), "thread-one")
            .expect("load session")
            .expect("session exists");
        let retained_sessions = store::list_all_sessions_from(temp_dir.path())
            .expect("list sessions")
            .into_iter()
            .map(|session| (session.session_id, session.ancestor_session_ids))
            .collect::<Vec<_>>();
        assert_eq!(
            (
                updated.status,
                updated.sweep_signaled,
                updated.archived_at,
                compacting_state(temp_dir.path(), "thread-one"),
                retained_sessions,
                sinks.removed.lock().expect("removed lock").clone(),
                sinks.archived.lock().expect("archive lock").clone(),
                sinks.synced.lock().expect("sync lock").clone(),
            ),
            (
                SessionStatus::Ended,
                false,
                None,
                (engine == Engine::Claude).then_some(false),
                vec![(
                    "thread-one".to_string(),
                    vec!["fixture-parent-session".to_string()],
                )],
                vec!["thread-one".to_string()],
                vec!["thread-one".to_string()],
                vec![(
                    Some("%42".to_string()),
                    Some(SessionStatus::Ended),
                    Some("thread-one".to_string()),
                    temp_dir.path().to_path_buf(),
                )],
            ),
        );
    }

    #[rstest]
    fn explicit_close_retries_when_compacting_state_cannot_be_cleared(
        lifecycle_test_context: LifecycleTestContext,
    ) {
        let LifecycleTestContext { temp_dir, sinks } = lifecycle_test_context;
        store::save_session_to(
            temp_dir.path(),
            &session(Engine::Claude, SessionStatus::Paused, false),
        )
        .expect("save paused session");
        store::set_watch_web_compacting(temp_dir.path(), "thread-one", true)
            .expect("stale compacting state should be set");
        let connection =
            rusqlite::Connection::open(temp_dir.path().join("sessions.db")).expect("open db");
        connection
            .execute_batch(
                "CREATE TRIGGER fail_compacting_reset
                 BEFORE UPDATE OF is_compacting ON watch_web_session_state
                 WHEN NEW.is_compacting = 0
                 BEGIN
                     SELECT RAISE(ABORT, 'compacting reset unavailable');
                 END;",
            )
            .expect("install compacting reset failure");

        let failed_close =
            mark_session_ended_after_close_in("thread-one", temp_dir.path(), &sinks.side_effects);
        let after_failure = store::load_session_from(temp_dir.path(), "thread-one")
            .expect("load session after failure")
            .expect("session exists after failure");
        let compacting_after_failure = compacting_state(temp_dir.path(), "thread-one");

        connection
            .execute_batch("DROP TRIGGER fail_compacting_reset;")
            .expect("remove compacting reset failure");
        mark_session_ended_after_close_in("thread-one", temp_dir.path(), &sinks.side_effects)
            .expect("close should succeed after storage recovers");

        let after_retry = store::load_session_from(temp_dir.path(), "thread-one")
            .expect("load session")
            .expect("session exists");
        assert_eq!(
            (
                failed_close.is_err(),
                after_failure.status,
                compacting_after_failure,
                after_retry.status,
                compacting_state(temp_dir.path(), "thread-one"),
                sinks.removed.lock().expect("removed lock").clone(),
                sinks.archived.lock().expect("archive lock").clone(),
                sinks.synced.lock().expect("sync lock").clone(),
            ),
            (
                true,
                SessionStatus::Paused,
                Some(true),
                SessionStatus::Ended,
                Some(false),
                vec!["thread-one".to_string()],
                vec!["thread-one".to_string()],
                vec![(
                    Some("%42".to_string()),
                    Some(SessionStatus::Ended),
                    Some("thread-one".to_string()),
                    temp_dir.path().to_path_buf(),
                )],
            ),
        );
    }
}
