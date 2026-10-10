use std::path::Path;

use anyhow::Result;
use chrono::Utc;

use super::super::{
    store,
    types::{Engine, SessionStatus},
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
                session_for_sync = Some(session.clone());
                return true;
            }
            SweepPauseOutcome::NotRequested => {}
        }

        session.status = SessionStatus::Stopped;
        session.updated_at = Utc::now();
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
        ended_session = Some(session.clone());
        true
    })?;

    if let Some(session) = ended_session {
        side_effects.remove_notification_group(session_id);
        side_effects.archive_tq_session(session_id);
        let work_type_config = session
            .work_type
            .as_ref()
            .map(|_| config::load_config_or_default());
        side_effects.sync_tmux(
            session.tmux_info.as_ref().map(|info| info.pane_id.as_str()),
            Some(SessionStatus::Ended),
            Some(&session),
            work_type_config.as_ref().map(|config| &config.agent),
            sessions_dir,
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::{Arc, Mutex};

    use chrono::Utc;
    use rstest::{fixture, rstest};
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
            session_id: "thread-one".to_string(),
            work_type: None,
            work_type_pinned: false,
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
            }),
            status,
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
            sweep_signaled,
            engine,
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
            ),
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
}
