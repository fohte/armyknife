use std::path::Path;

#[cfg(not(test))]
use super::super::archive_tq_session_detached;
use super::super::tmux_sync::{LiveTmuxStatusSyncer, TmuxStatusSyncer};
use super::super::types::{Session, SessionStatus};
use crate::shared::config::AgentConfig;

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
    /// Spawn the detached `a agent archive-tq-session-detached` worker on a
    /// genuine Ended transition. Off in tests (would fork a real process).
    pub(super) tq_archive: bool,
    /// Include `a agent bg run` state in Stop processing.
    pub(super) track_bg_run_tasks: bool,
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

impl SideEffects {
    pub(super) fn all() -> Self {
        Self {
            tmux: true,
            notifications: true,
            auto_compact: true,
            tq_archive: true,
            track_bg_run_tasks: true,
            #[cfg(test)]
            removed_notification_groups: None,
            #[cfg(test)]
            tmux_sync_calls: None,
            #[cfg(test)]
            tq_archive_calls: None,
        }
    }

    #[cfg(test)]
    pub(super) fn none() -> Self {
        Self {
            tmux: false,
            notifications: false,
            auto_compact: false,
            tq_archive: false,
            track_bg_run_tasks: false,
            removed_notification_groups: None,
            tmux_sync_calls: None,
            tq_archive_calls: None,
        }
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
