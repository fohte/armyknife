//! Trait abstraction over the "push the latest Claude Code status into tmux
//! pane / window user options" side effect, so that hook-driven and
//! sweep-driven paths share a single implementation and tests can verify the
//! call without touching a real tmux server.
//!
//! See `pane_status::sync_pane_option` / `window_status::sync_window_option`
//! for what the production implementation actually writes.

use std::path::Path;

use super::pane_status;
use super::types::SessionStatus;
use super::window_status;
use crate::infra::tmux;

/// Pushes the aggregated window status for the window containing `pane_id`
/// and the pane's own prompt-indicator status into their respective tmux user
/// options.
pub(crate) trait TmuxStatusSyncer {
    fn sync(&self, pane_id: Option<&str>, status: Option<SessionStatus>, sessions_dir: &Path);
}

/// Production syncer that drives the real tmux server.
///
/// No-op when there is no pane (session ran outside tmux). The pane-level
/// write does not depend on resolving a window, so it still runs when the
/// window lookup fails. Errors are ignored: both options are best-effort.
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
        let _ = pane_status::sync_pane_option(pane_id, status, sessions_dir);
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
