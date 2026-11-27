use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::Result;
use axum::Json;
use axum::extract::{Path as RoutePath, State};
use chrono::{DateTime, Utc};
use tokio::sync::Mutex;
use tokio::time::Instant;

use super::super::super::types::{Session, SessionStatus};
use super::{ActionState, ApiError, Success, load_session, resolve_current_session_id};
use crate::commands::agent::watch_web::EnrichmentUpdates;
use crate::infra::tmux::layout::ResumeLayoutSpec;
use crate::shared::config::LayoutNode;

const RESUME_START_TIMEOUT: Duration = Duration::from_secs(20);

pub(super) trait EndedSessionHost {
    fn open_layout(&self, spec: ResumeLayoutSpec<'_>) -> anyhow::Result<()>;
}

pub(super) struct TmuxEndedSessionHost;

impl EndedSessionHost for TmuxEndedSessionHost {
    fn open_layout(&self, spec: ResumeLayoutSpec<'_>) -> anyhow::Result<()> {
        crate::infra::tmux::layout::build_resume_layout(spec).map(|_| ())
    }
}

pub(super) fn resume_ended_session(
    session: &Session,
    layout: &LayoutNode,
    host: &impl EndedSessionHost,
) -> anyhow::Result<()> {
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
        resume_marker: DateTime<Utc>,
    },
    Stuck {
        resume_marker: DateTime<Utc>,
    },
}

impl ResumeStartupState {
    fn resume_marker(&self) -> &DateTime<Utc> {
        match self {
            Self::Pending { resume_marker, .. } | Self::Stuck { resume_marker } => resume_marker,
        }
    }
}

#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ResumeStartupStatus {
    Pending,
    Stuck,
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
        updated_at: &DateTime<Utc>,
    ) -> bool {
        let mut states = self.states.lock().await;
        let Some(state) = states.get(session_id).cloned() else {
            return false;
        };
        if updated_at > state.resume_marker()
            || !matches!(status, SessionStatus::Ended | SessionStatus::Paused)
        {
            states.remove(session_id);
            return false;
        }

        matches!(
            state,
            ResumeStartupState::Pending { deadline, .. } if deadline > Instant::now()
        )
    }

    async fn mark_pending(&self, session_id: String, resume_marker: DateTime<Utc>) {
        let attempt_id = self.next_attempt_id.fetch_add(1, Ordering::Relaxed);
        let deadline = Instant::now() + RESUME_START_TIMEOUT;
        self.states.lock().await.insert(
            session_id.clone(),
            ResumeStartupState::Pending {
                attempt_id,
                deadline,
                resume_marker,
            },
        );
        self.updates.notify();

        let resumes = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep_until(deadline).await;
            if resumes.mark_stuck(&session_id, attempt_id).await {
                resumes.updates.notify();
            }
        });
    }

    async fn mark_stuck(&self, session_id: &str, attempt_id: u64) -> bool {
        let mut states = self.states.lock().await;
        let Some(ResumeStartupState::Pending {
            attempt_id: current_attempt_id,
            resume_marker,
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
            ResumeStartupState::Stuck { resume_marker },
        );
        true
    }

    pub(super) async fn startup_status(
        &self,
        session_id: &str,
        status: SessionStatus,
        updated_at: &DateTime<Utc>,
    ) -> Option<ResumeStartupStatus> {
        let mut states = self.states.lock().await;
        let state = states.get(session_id).cloned()?;
        if updated_at > state.resume_marker()
            || !matches!(status, SessionStatus::Ended | SessionStatus::Paused)
        {
            states.remove(session_id);
            return None;
        }

        match state {
            ResumeStartupState::Pending { .. } => Some(ResumeStartupStatus::Pending),
            ResumeStartupState::Stuck { .. } => Some(ResumeStartupStatus::Stuck),
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
    let session = load_session(state.clone(), current_id.clone()).await?;
    if state
        .resume_pending
        .is_pending(&current_id, session.status, &session.updated_at)
        .await
    {
        return Err(ApiError::conflict("session resume is already in progress"));
    }
    if !matches!(session.status, SessionStatus::Ended | SessionStatus::Paused) {
        return Err(ApiError::conflict("session is not ended or paused"));
    }

    // SessionStart may update this timestamp before the blocking resume call returns.
    let resume_marker = session.updated_at;
    let runtime = state.runtime;
    tokio::task::spawn_blocking(move || runtime.resume_session(&session))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::operation)?;
    state
        .resume_pending
        .mark_pending(current_id, resume_marker)
        .await;

    Ok(Json(Success::new()))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
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
        fn open_layout(&self, spec: ResumeLayoutSpec<'_>) -> Result<()> {
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
            Ok(())
        }
    }

    #[test]
    fn resume_ended_session_passes_the_saved_context_to_the_layout() {
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
        session.cwd = PathBuf::from("/fixture/worktree");
        session.tmux_info = Some(TmuxInfo {
            session_name: "fixture-session-window".to_string(),
            window_name: "fixture-agent".to_string(),
            window_index: 2,
            pane_id: "%17".to_string(),
            socket_path: Some("/fixture/tmux.sock".to_string()),
        });
        let host = FakeEndedSessionHost::default();
        let layout = LayoutNode::default();

        resume_ended_session(&session, &layout, &host).unwrap();

        assert_eq!(
            *host.calls.lock().unwrap(),
            vec![json!({
                "kind": "layout",
                "socketPath": "/fixture/tmux.sock",
                "sessionName": "fixture-session-window",
                "agentSessionId": "fixture-session",
                "cwd": "/fixture/worktree",
                "windowName": "fixture-agent",
                "engine": "codex",
                "agentCommand": "a agent resume fixture-session --engine codex --ancestor-session-ids fixture-parent --launched-with-vibe",
                "layout": {
                    "direction": "horizontal",
                    "first": { "command": "nvim", "focus": true },
                    "second": { "command": "claude", "focus": false },
                },
            }),],
        );
    }
}
