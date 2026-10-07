//! Trait abstraction over the pane and window tmux status side effects, so
//! hook-driven and sweep-driven paths share one implementation and tests can
//! verify the call without touching a real tmux server or the process temp dir.
//!
//! The production implementation writes pane options, a paused-flag file,
//! and the aggregated window options.

use std::path::Path;

use super::pane;
use super::pane_options;
use super::store;
use super::types::{SessionStatus, resolve_session_option};
use super::window_status;
use crate::infra::tmux;
use crate::shared::config;

/// Pushes pane options, its paused-flag file, and status into the containing
/// window's tmux options.
pub(crate) trait TmuxStatusSyncer {
    fn sync(&self, pane_id: Option<&str>, status: Option<SessionStatus>, sessions_dir: &Path);
}

/// Production syncer that drives the real tmux server.
///
/// No-op when there is no pane (session ran outside tmux). Pane writes do not
/// depend on resolving a window, so they still run when lookup fails. Errors
/// are ignored: all writes are best-effort.
///
/// Only the window the pane *currently* belongs to is recomputed. Moving a
/// pane across windows (`move-pane` / `break-pane`) leaves the source
/// window's option stale until one of its own sessions next fires a hook --
/// rare enough not to warrant tracking each pane's previous window.
pub(crate) struct LiveTmuxStatusSyncer;

impl TmuxStatusSyncer for LiveTmuxStatusSyncer {
    fn sync(&self, pane_id: Option<&str>, status: Option<SessionStatus>, sessions_dir: &Path) {
        let Some(pane_id) = pane_id else {
            return;
        };
        let _ = pane::status::sync_paused_flag(pane_id, status, sessions_dir);
        let session = resolve_session_option(|option| tmux::get_pane_option(pane_id, option))
            .and_then(|session_id| {
                store::load_session_from(sessions_dir, &session_id)
                    .ok()
                    .flatten()
            });
        let agent_config = if session
            .as_ref()
            .is_some_and(|session| session.work_type.is_some())
        {
            config::load_config_or_default().agent
        } else {
            crate::shared::config::AgentConfig::default()
        };
        let _ = tmux::run_batch(&pane_options::tmux_option_commands(
            pane_id,
            session.as_ref(),
            &agent_config,
        ));
        let Some(window_id) = tmux::get_window_id_for_pane(pane_id) else {
            return;
        };
        let _ = window_status::sync_window_option(&window_id, sessions_dir);
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    use super::super::types::SessionStatus;
    use super::TmuxStatusSyncer;

    pub(crate) type SyncCall = (Option<String>, Option<SessionStatus>, PathBuf);

    /// Test double that records every `sync` call.
    #[derive(Default)]
    pub(crate) struct RecordingTmuxStatusSyncer {
        pub calls: RefCell<Vec<SyncCall>>,
    }

    impl TmuxStatusSyncer for RecordingTmuxStatusSyncer {
        fn sync(&self, pane_id: Option<&str>, status: Option<SessionStatus>, sessions_dir: &Path) {
            self.calls.borrow_mut().push((
                pane_id.map(str::to_string),
                status,
                sessions_dir.to_path_buf(),
            ));
        }
    }
}
