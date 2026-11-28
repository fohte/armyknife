use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Result, ensure};
use axum::Json;
use axum::extract::{Path as RoutePath, State};
use chrono::{DateTime, Utc};
use tokio::sync::Mutex;
use tokio::time::Instant;

use super::super::super::types::{Session, SessionStatus};
use super::super::store;
use super::{
    ActionState, ApiError, Success, load_session, load_session_start_generation,
    resolve_current_session_id, resolve_sessions_dir, run_blocking,
};
use crate::commands::agent::watch_web::EnrichmentUpdates;
use crate::infra::tmux::layout::ResumeLayoutSpec;
use crate::shared::config::LayoutNode;

const RESUME_START_TIMEOUT: Duration = Duration::from_secs(20);

pub(super) trait EndedSessionHost {
    fn open_layout(&self, spec: ResumeLayoutSpec<'_>) -> anyhow::Result<String>;
}

pub(super) struct TmuxEndedSessionHost;

impl EndedSessionHost for TmuxEndedSessionHost {
    fn open_layout(&self, spec: ResumeLayoutSpec<'_>) -> anyhow::Result<String> {
        crate::infra::tmux::layout::build_resume_layout(spec)
    }
}

pub(super) fn resume_ended_session(
    session: &Session,
    layout: &LayoutNode,
    host: &impl EndedSessionHost,
) -> anyhow::Result<String> {
    ensure!(
        session.cwd.is_dir(),
        "session working directory does not exist: {}",
        session.cwd.display()
    );
    let tmux_info = session
        .tmux_info
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("session has no tmux location"))?;
    let command = crate::commands::agent::resurrect::resume_command_for_session(session);
    let cwd = session.cwd.to_string_lossy();
    host.open_layout(ResumeLayoutSpec {
        session: &tmux_info.session_name,
        agent_session_id: &session.session_id,
        cwd: cwd.as_ref(),
        window_name: &tmux_info.window_name,
        layout,
        engine: session.engine,
        agent_command: &command,
        socket_path: tmux_info.socket_path.as_deref(),
    })
}

#[derive(Clone)]
pub(super) struct PendingSessionResumes {
    states: Arc<Mutex<HashMap<String, ResumeStartupState>>>,
    next_attempt_id: Arc<AtomicU64>,
    updates: EnrichmentUpdates,
}

#[derive(Clone)]
enum ResumeStartupState {
    Pending {
        attempt_id: u64,
        deadline: Instant,
        session_start_generation: i64,
        started_at: DateTime<Utc>,
    },
    Stuck {
        session_start_generation: i64,
        started_at: DateTime<Utc>,
    },
}

impl ResumeStartupState {
    fn session_start_generation(&self) -> i64 {
        match self {
            Self::Pending {
                session_start_generation,
                ..
            }
            | Self::Stuck {
                session_start_generation,
                ..
            } => *session_start_generation,
        }
    }
}

#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ResumeStartupStatus {
    Pending,
    Stuck,
}

fn is_awaiting_session_start(status: SessionStatus) -> bool {
    // Codex marks a resumed session Stopped before its SessionStart hook advances the generation.
    matches!(
        status,
        SessionStatus::Ended | SessionStatus::Paused | SessionStatus::Stopped
    )
}

impl PendingSessionResumes {
    pub(super) fn new(updates: EnrichmentUpdates) -> Self {
        Self {
            states: Arc::default(),
            next_attempt_id: Arc::default(),
            updates,
        }
    }

    async fn is_pending(
        &self,
        session_id: &str,
        status: SessionStatus,
        session_start_generation: i64,
    ) -> bool {
        let mut states = self.states.lock().await;
        let Some(state) = states.get(session_id).cloned() else {
            return false;
        };
        if session_start_generation > state.session_start_generation()
            || !is_awaiting_session_start(status)
        {
            states.remove(session_id);
            return false;
        }

        matches!(
            state,
            ResumeStartupState::Pending { deadline, .. } if deadline > Instant::now()
        )
    }

    pub(super) async fn mark_pending(
        &self,
        session_id: String,
        session_start_generation: i64,
    ) -> u64 {
        let attempt_id = self.next_attempt_id.fetch_add(1, Ordering::Relaxed);
        let deadline = Instant::now() + RESUME_START_TIMEOUT;
        let started_at = Utc::now();
        self.states.lock().await.insert(
            session_id.clone(),
            ResumeStartupState::Pending {
                attempt_id,
                deadline,
                session_start_generation,
                started_at,
            },
        );
        self.updates.notify();

        let resumes = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep_until(deadline).await;
            resumes.mark_stuck(&session_id, attempt_id).await;
        });
        attempt_id
    }

    pub(super) async fn mark_stuck(&self, session_id: &str, attempt_id: u64) -> bool {
        let mut states = self.states.lock().await;
        let Some(ResumeStartupState::Pending {
            attempt_id: current_attempt_id,
            session_start_generation,
            started_at,
            ..
        }) = states.get(session_id).cloned()
        else {
            return false;
        };
        if current_attempt_id != attempt_id {
            return false;
        }

        states.insert(
            session_id.to_string(),
            ResumeStartupState::Stuck {
                session_start_generation,
                started_at,
            },
        );
        drop(states);
        self.updates.notify();
        true
    }

    pub(super) async fn startup_status(
        &self,
        session_id: &str,
        status: SessionStatus,
        session_start_generation: i64,
    ) -> Option<(ResumeStartupStatus, DateTime<Utc>)> {
        let mut states = self.states.lock().await;
        let state = states.get(session_id).cloned()?;
        if session_start_generation > state.session_start_generation()
            || !is_awaiting_session_start(status)
        {
            states.remove(session_id);
            return None;
        }

        match state {
            ResumeStartupState::Pending { started_at, .. } => {
                Some((ResumeStartupStatus::Pending, started_at))
            }
            ResumeStartupState::Stuck { started_at, .. } => {
                Some((ResumeStartupStatus::Stuck, started_at))
            }
        }
    }
}

pub(super) async fn resume_session(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
) -> Result<Json<Success>, ApiError> {
    load_session(state.clone(), id.clone()).await?;
    let current_id = resolve_current_session_id(&state, &id).await?;
    let lock = state.resume_locks.get_or_create(current_id.clone()).await;
    let _guard = lock
        .try_lock_owned()
        .map_err(|_| ApiError::conflict("session resume is already in progress"))?;
    let session_start_generation = load_session_start_generation(&state, &current_id).await?;
    // SessionStart writes this marker with the session row, so reading it first avoids pairing a
    // newer marker with a stale resumable status.
    let session = load_session(state.clone(), current_id.clone()).await?;
    if state
        .resume_pending
        .is_pending(&current_id, session.status, session_start_generation)
        .await
    {
        return Err(ApiError::conflict("session resume is already in progress"));
    }
    if !matches!(session.status, SessionStatus::Ended | SessionStatus::Paused) {
        return Err(ApiError::conflict("session is not ended or paused"));
    }

    let runtime = state.runtime;
    let pane_id = tokio::task::spawn_blocking(move || {
        ensure!(
            session.cwd.is_dir(),
            "session working directory does not exist: {}",
            session.cwd.display()
        );
        runtime.resume_session(&session)
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(ApiError::operation)?;
    if let Some(pane_id) = pane_id {
        let sessions_dir =
            resolve_sessions_dir(state.sessions_dir.clone()).map_err(ApiError::internal)?;
        let session_id = current_id.clone();
        run_blocking(move || {
            store::update_session_in(&sessions_dir, &session_id, |session| {
                let Some(tmux_info) = session.tmux_info.as_mut() else {
                    return false;
                };
                tmux_info.pane_id = pane_id;
                true
            })
        })
        .await?;
    }
    state
        .resume_pending
        .mark_pending(current_id, session_start_generation)
        .await;

    Ok(Json(Success::new()))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use anyhow::Result;
    use serde_json::{Value, json};

    use super::{EndedSessionHost, resume_ended_session};
    use crate::commands::agent::types::{Engine, Session, TmuxInfo};
    use crate::infra::tmux::layout::ResumeLayoutSpec;
    use crate::shared::config::LayoutNode;

    #[derive(Default)]
    struct FakeEndedSessionHost {
        calls: StdMutex<Vec<Value>>,
    }

    impl EndedSessionHost for FakeEndedSessionHost {
        fn open_layout(&self, spec: ResumeLayoutSpec<'_>) -> Result<String> {
            self.calls.lock().unwrap().push(json!({
                "kind": "layout",
                "socketPath": spec.socket_path,
                "sessionName": spec.session,
                "agentSessionId": spec.agent_session_id,
                "cwd": spec.cwd,
                "windowName": spec.window_name,
                "engine": spec.engine,
                "agentCommand": spec.agent_command,
                "layout": serde_json::to_value(spec.layout).unwrap(),
            }));
            Ok("%18".to_string())
        }
    }

    #[test]
    fn resume_ended_session_passes_the_saved_context_to_the_layout() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut session: Session = serde_json::from_value(json!({
            "session_id": "fixture-session",
            "cwd": "/fixture/worktree",
            "transcript_path": null,
            "tmux_info": null,
            "status": "ended",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_message": null,
        }))
        .unwrap();
        session.engine = Engine::Codex;
        session.ancestor_session_ids = vec!["fixture-parent".to_string()];
        session.launched_with_vibe = true;
        session.cwd = temp_dir.path().to_path_buf();
        session.tmux_info = Some(TmuxInfo {
            session_name: "fixture-session-window".to_string(),
            window_name: "fixture-agent".to_string(),
            window_index: 2,
            pane_id: "%17".to_string(),
            socket_path: Some("/fixture/tmux.sock".to_string()),
        });
        let host = FakeEndedSessionHost::default();
        let layout = LayoutNode::default();

        let pane_id = resume_ended_session(&session, &layout, &host).unwrap();

        assert_eq!(
            (pane_id, host.calls.lock().unwrap().clone()),
            (
                "%18".to_string(),
                vec![json!({
                    "kind": "layout",
                    "socketPath": "/fixture/tmux.sock",
                    "sessionName": "fixture-session-window",
                    "agentSessionId": "fixture-session",
                    "cwd": temp_dir.path().display().to_string(),
                    "windowName": "fixture-agent",
                    "engine": "codex",
                    "agentCommand": "a agent resume fixture-session --engine codex --ancestor-session-ids fixture-parent --launched-with-vibe",
                    "layout": {
                        "direction": "horizontal",
                        "first": { "command": "nvim", "focus": true },
                        "second": { "command": "claude", "focus": false },
                    },
                }),]
            ),
        );
    }

    #[test]
    fn resume_ended_session_rejects_a_missing_working_directory_before_opening_a_layout() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut session: Session = serde_json::from_value(json!({
            "session_id": "fixture-session",
            "cwd": temp_dir.path().join("missing-worktree"),
            "transcript_path": null,
            "tmux_info": null,
            "status": "ended",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_message": null,
        }))
        .unwrap();
        session.tmux_info = Some(TmuxInfo {
            session_name: "fixture-session-window".to_string(),
            window_name: "fixture-agent".to_string(),
            window_index: 0,
            pane_id: "%17".to_string(),
            socket_path: Some("/fixture/tmux.sock".to_string()),
        });
        let host = FakeEndedSessionHost::default();
        let layout = LayoutNode::default();

        let result = resume_ended_session(&session, &layout, &host);

        assert_eq!(
            json!({
                "error": result.unwrap_err().to_string(),
                "calls": host.calls.lock().unwrap().clone(),
            }),
            json!({
                "error": format!(
                    "session working directory does not exist: {}",
                    session.cwd.display(),
                ),
                "calls": [],
            }),
        );
    }
}
