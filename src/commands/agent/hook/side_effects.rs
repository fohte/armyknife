use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use super::super::status_inference::StatusInferenceSpawnRequest;
use super::super::tmux_sync::{LiveTmuxStatusSyncer, TmuxStatusSyncer};
#[cfg(test)]
use super::super::types::HookEvent;
use super::super::types::{Session, SessionPrContext, SessionStatus};
#[cfg(not(test))]
use super::super::{archive_tq_session_detached, bg};
use crate::shared::config::AgentConfig;

pub(super) type PrContextCapture = Arc<dyn Fn(&Path) -> Option<SessionPrContext> + Send + Sync>;

/// Controls which side effects `process_hook_event_impl` executes.
/// Production code uses `SideEffects::all()`; tests use `SideEffects::none()`
/// to avoid external commands and the shared background-task registry.
pub(super) struct SideEffects {
    /// Call tmux commands (get_pane_info_by_pid, set_pane_option, refresh_status)
    pub(super) tmux: bool,
    /// Send/remove notifications via hammerspoon
    pub(super) notifications: bool,
    /// Spawn the detached `a agent auto-compact schedule` worker on Stop events.
    /// Off in tests (would fork a real process and survive past the test).
    pub(super) auto_compact: bool,
    /// Spawn the detached status inference worker on eligible Stop events.
    /// Off in tests (would fork a real process and survive past the test).
    pub(super) status_inference: bool,
    pub(super) capture_pr_context: Option<PrContextCapture>,
    /// Spawn the detached `a agent archive-tq-session-detached` worker on a
    /// genuine Ended transition. Off in tests (would fork a real process).
    pub(super) tq_archive: bool,
    /// Include `a agent bg run` state in Stop processing.
    pub(super) track_bg_run_tasks: bool,
    /// Keep SessionStart(resume) from waiting for message delivery readiness.
    pub(super) redeliver_bg_notifications: bool,
    /// Test-only sink that records the group ids passed to
    /// `remove_notification_group`. Lets tests assert the call happened
    /// without invoking hammerspoon.
    #[cfg(test)]
    pub(super) removed_notification_groups: Option<std::sync::Arc<std::sync::Mutex<Vec<String>>>>,
    /// Test-only sink that records (pane_id, status, session_id, sessions_dir) tuples
    /// passed to `sync_tmux`. Lets tests assert the call happened with the
    /// expected status without invoking tmux.
    #[cfg(test)]
    pub(super) tmux_sync_calls: Option<TmuxSyncCallSink>,
    /// Test-only sink that records session ids passed to the detached tq
    /// archive worker.
    #[cfg(test)]
    pub(super) tq_archive_calls: Option<std::sync::Arc<std::sync::Mutex<Vec<String>>>>,
    /// Test-only sink that records notification events without invoking an external service.
    #[cfg(test)]
    pub(super) notification_calls: Option<std::sync::Arc<std::sync::Mutex<Vec<HookEvent>>>>,
    /// Test-only sink that records sessions passed to the auto-compact worker.
    #[cfg(test)]
    pub(super) auto_compact_calls: Option<std::sync::Arc<std::sync::Mutex<Vec<String>>>>,
    /// Test-only sink that records each Stop snapshot passed to the worker.
    #[cfg(test)]
    pub(super) status_inference_calls: Option<StatusInferenceCallSink>,
    /// Test-only failure injected at the detached status inference spawn boundary.
    #[cfg(test)]
    pub(super) status_inference_spawn_error: Option<String>,
    /// Test-only sink that records sessions passed to the redelivery worker.
    #[cfg(test)]
    pub(super) redelivery_calls: Option<std::sync::Arc<std::sync::Mutex<Vec<String>>>>,
}

#[cfg(test)]
type TmuxSyncCall = (
    Option<String>,
    Option<SessionStatus>,
    Option<String>,
    std::path::PathBuf,
);
#[cfg(test)]
type TmuxSyncCallSink = std::sync::Arc<std::sync::Mutex<Vec<TmuxSyncCall>>>;
#[cfg(test)]
type StatusInferenceCallSink = std::sync::Arc<std::sync::Mutex<Vec<StatusInferenceSpawnRequest>>>;

impl SideEffects {
    pub(super) fn all() -> Self {
        Self {
            tmux: true,
            notifications: true,
            auto_compact: true,
            status_inference: true,
            capture_pr_context: Some(Arc::new(super::super::session_pr_context::capture)),
            tq_archive: true,
            track_bg_run_tasks: true,
            redeliver_bg_notifications: true,
            #[cfg(test)]
            removed_notification_groups: None,
            #[cfg(test)]
            tmux_sync_calls: None,
            #[cfg(test)]
            tq_archive_calls: None,
            #[cfg(test)]
            notification_calls: None,
            #[cfg(test)]
            auto_compact_calls: None,
            #[cfg(test)]
            status_inference_calls: None,
            #[cfg(test)]
            status_inference_spawn_error: None,
            #[cfg(test)]
            redelivery_calls: None,
        }
    }

    #[cfg(test)]
    pub(super) fn none() -> Self {
        Self {
            tmux: false,
            notifications: false,
            auto_compact: false,
            status_inference: false,
            capture_pr_context: None,
            tq_archive: false,
            track_bg_run_tasks: false,
            redeliver_bg_notifications: false,
            removed_notification_groups: None,
            tmux_sync_calls: None,
            tq_archive_calls: None,
            notification_calls: None,
            auto_compact_calls: None,
            status_inference_calls: None,
            status_inference_spawn_error: None,
            redelivery_calls: None,
        }
    }

    #[cfg(test)]
    pub(super) fn send_notification(&self, event: HookEvent) {
        if let Some(calls) = &self.notification_calls {
            calls
                .lock()
                .expect("notification_calls mutex poisoned")
                .push(event);
        }
    }

    pub(super) fn schedule_auto_compact(&self, session_id: &str) {
        #[cfg(test)]
        if let Some(calls) = &self.auto_compact_calls {
            calls
                .lock()
                .expect("auto_compact_calls mutex poisoned")
                .push(session_id.to_string());
        }
        #[cfg(not(test))]
        super::super::auto_compact::spawn_in_background(session_id);
    }

    pub(super) fn schedule_status_inference(
        &self,
        request: &StatusInferenceSpawnRequest,
    ) -> Result<()> {
        if !self.status_inference {
            return Ok(());
        }
        #[cfg(test)]
        if let Some(calls) = &self.status_inference_calls {
            calls
                .lock()
                .expect("status_inference_calls mutex poisoned")
                .push(request.clone());
            if let Some(error) = &self.status_inference_spawn_error {
                return Err(anyhow::anyhow!(error.clone()));
            }
            return Ok(());
        }
        #[cfg(not(test))]
        return super::super::status_inference::spawn_in_background(request);
        #[cfg(test)]
        Ok(())
    }

    pub(super) fn redeliver_bg_notifications(&self, session_id: &str) {
        if !self.redeliver_bg_notifications {
            return;
        }
        #[cfg(test)]
        if let Some(calls) = &self.redelivery_calls {
            calls
                .lock()
                .expect("redelivery_calls mutex poisoned")
                .push(session_id.to_string());
        }
        #[cfg(not(test))]
        bg::spawn_redelivery_if_pending(session_id);
    }

    /// Pushes the latest pane / window status into tmux. In tests, also
    /// records the call into `tmux_sync_calls` so assertions don't require
    /// real tmux.
    pub(super) fn sync_tmux(
        &self,
        pane_id: Option<&str>,
        status: Option<SessionStatus>,
        session: Option<&Session>,
        agent_config: Option<&AgentConfig>,
        sessions_dir: &Path,
    ) {
        #[cfg(test)]
        if let Some(rec) = &self.tmux_sync_calls {
            rec.lock().expect("tmux_sync_calls mutex poisoned").push((
                pane_id.map(str::to_string),
                status,
                session.map(|session| session.session_id.clone()),
                sessions_dir.to_path_buf(),
            ));
        }
        if self.tmux {
            LiveTmuxStatusSyncer.sync(pane_id, status, session, agent_config, sessions_dir);
        }
    }

    pub(super) fn remove_notification_group(&self, group: &str) {
        #[cfg(not(test))]
        if self.notifications {
            let _ = crate::infra::notification::remove_group(group);
        }
        #[cfg(test)]
        if let Some(rec) = &self.removed_notification_groups {
            rec.lock()
                .expect("removed_notification_groups mutex poisoned")
                .push(group.to_string());
        }
    }

    pub(super) fn archive_tq_session(&self, session_id: &str) {
        if !self.tq_archive {
            return;
        }
        #[cfg(test)]
        if let Some(calls) = &self.tq_archive_calls {
            calls
                .lock()
                .expect("tq_archive_calls mutex poisoned")
                .push(session_id.to_string());
        }
        #[cfg(not(test))]
        archive_tq_session_detached::spawn_in_background(session_id);
    }
}
