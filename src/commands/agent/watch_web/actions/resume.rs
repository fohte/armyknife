use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use axum::Json;
use axum::extract::{Path as RoutePath, State};
use tokio::sync::Mutex;

use super::super::super::types::{Session, SessionStatus};
use super::{ActionState, ApiError, Success, load_session};
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
        cwd: cwd.as_ref(),
        window_name: &tmux_info.window_name,
        layout,
        engine: session.engine,
        agent_command: &command,
        socket_path: tmux_info.socket_path.as_deref(),
    })
}

#[derive(Clone, Default)]
pub(super) struct PendingSessionResumes(Arc<Mutex<HashMap<String, Instant>>>);

impl PendingSessionResumes {
    async fn is_pending(&self, session_id: &str, status: SessionStatus) -> bool {
        let mut pending = self.0.lock().await;
        if !matches!(status, SessionStatus::Ended | SessionStatus::Paused) {
            pending.remove(session_id);
            return false;
        }

        let Some(expires_at) = pending.get(session_id).copied() else {
            return false;
        };
        if expires_at > Instant::now() {
            return true;
        }

        pending.remove(session_id);
        false
    }

    async fn mark_pending(&self, session_id: String) {
        self.0
            .lock()
            .await
            .insert(session_id, Instant::now() + RESUME_START_TIMEOUT);
    }

    pub(super) async fn clear_if_not_resumable(&self, session_id: &str, status: SessionStatus) {
        if matches!(status, SessionStatus::Ended | SessionStatus::Paused) {
            return;
        }
        self.0.lock().await.remove(session_id);
    }
}

pub(super) async fn resume_session(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
) -> Result<Json<Success>, ApiError> {
    let lock = state.resume_locks.get_or_create(id.clone()).await;
    let _guard = lock
        .try_lock_owned()
        .map_err(|_| ApiError::conflict("session resume is already in progress"))?;
    let session = load_session(state.clone(), id.clone()).await?;
    if state.resume_pending.is_pending(&id, session.status).await {
        return Err(ApiError::conflict("session resume is already in progress"));
    }
    if !matches!(session.status, SessionStatus::Ended | SessionStatus::Paused) {
        return Err(ApiError::conflict("session is not ended or paused"));
    }

    let runtime = state.runtime;
    tokio::task::spawn_blocking(move || runtime.resume_session(&session))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::operation)?;
    state.resume_pending.mark_pending(id).await;

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
