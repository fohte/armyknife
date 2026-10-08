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
use super::types::{Session, SessionStatus, resolve_session_option};
use super::window_status;
use crate::infra::tmux;
use crate::shared::config::{self, AgentConfig};

/// Synchronizes pane options, the paused marker, and the containing window's
/// aggregated status.
pub(crate) trait TmuxStatusSyncer {
    fn sync(
        &self,
        pane_id: Option<&str>,
        status: Option<SessionStatus>,
        session: Option<&Session>,
        agent_config: Option<&AgentConfig>,
        sessions_dir: &Path,
    );
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
    fn sync(
        &self,
        pane_id: Option<&str>,
        status: Option<SessionStatus>,
        session: Option<&Session>,
        agent_config: Option<&AgentConfig>,
        sessions_dir: &Path,
    ) {
        let Some(pane_id) = pane_id else {
            return;
        };
        let _ = pane::status::sync_paused_flag(pane_id, status, sessions_dir);
        let loaded_session = if session.is_none() {
            resolve_session_option(|option| tmux::get_pane_option(pane_id, option)).and_then(
                |session_id| {
                    store::load_session_from(sessions_dir, &session_id)
                        .ok()
                        .flatten()
                },
            )
        } else {
            None
        };
        let session = session.or(loaded_session.as_ref());
        let loaded_config = (agent_config.is_none()
            && session.is_some_and(|session| session.work_type.is_some()))
        .then(|| config::load_config_or_default().agent);
        let default_config = AgentConfig::default();
        let agent_config = agent_config
            .or(loaded_config.as_ref())
            .unwrap_or(&default_config);
        sync_pane_options(pane_id, session, agent_config, tmux::run_batch);
        let Some(window_id) = tmux::get_window_id_for_pane(pane_id) else {
            return;
        };
        let _ = window_status::sync_window_option(&window_id, sessions_dir);
    }
}

fn sync_pane_options(
    pane_id: &str,
    session: Option<&Session>,
    agent_config: &AgentConfig,
    run_batch: impl FnOnce(&[Vec<String>]) -> tmux::Result<()>,
) {
    let commands = pane_options::tmux_option_commands(pane_id, session, agent_config);
    if let Err(error) = run_batch(&commands) {
        tracing::warn!(event = "agent.tmux.pane_options.sync_failed", pane_id, %error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_pane_options_sends_unsets_when_session_is_unavailable() {
        let mut applied_commands = Vec::new();
        sync_pane_options("%42", None, &AgentConfig::default(), |commands| {
            applied_commands.extend_from_slice(commands);
            Ok(())
        });

        assert_eq!(
            applied_commands,
            vec![
                vec![
                    "set-option",
                    "-p",
                    "-u",
                    "-t",
                    "%42",
                    "@armyknife-cc-pane-status",
                ],
                vec![
                    "set-option",
                    "-p",
                    "-u",
                    "-t",
                    "%42",
                    "@armyknife-cc-pane-work-type",
                ],
            ],
        );
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    use super::super::types::SessionStatus;
    use super::TmuxStatusSyncer;
    use crate::shared::config::AgentConfig;

    pub(crate) type SyncCall = (
        Option<String>,
        Option<SessionStatus>,
        Option<String>,
        PathBuf,
    );

    /// Test double that records every `sync` call.
    #[derive(Default)]
    pub(crate) struct RecordingTmuxStatusSyncer {
        pub calls: RefCell<Vec<SyncCall>>,
    }

    impl TmuxStatusSyncer for RecordingTmuxStatusSyncer {
        fn sync(
            &self,
            pane_id: Option<&str>,
            status: Option<SessionStatus>,
            session: Option<&super::super::types::Session>,
            _agent_config: Option<&AgentConfig>,
            sessions_dir: &Path,
        ) {
            self.calls.borrow_mut().push((
                pane_id.map(str::to_string),
                status,
                session.map(|session| session.session_id.clone()),
                sessions_dir.to_path_buf(),
            ));
        }
    }
}
