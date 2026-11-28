use std::path::Path;
use std::sync::Arc;

use super::super::tmux_sync::{LiveTmuxStatusSyncer, TmuxStatusSyncer};
#[cfg(test)]
use super::super::types::HookEvent;
use super::super::types::{
    HookInput, Session, SessionPrContext, SessionStatus, TMUX_SESSION_OPTION,
};
#[cfg(not(test))]
use super::super::{archive_tq_session_detached, bg};
use crate::infra::tmux;
use crate::shared::config::AgentConfig;

pub(super) type PrContextCapture = Arc<dyn Fn(&Path) -> Option<SessionPrContext> + Send + Sync>;

#[cfg(test)]
type TitleGenerationCall = (String, String, Option<String>, bool);
#[cfg(test)]
type TitleGenerationCallSink = std::sync::Arc<std::sync::Mutex<Vec<TitleGenerationCall>>>;

#[cfg(test)]
type TranscriptSlimCall = (String, std::path::PathBuf, Vec<String>);
#[cfg(test)]
type TranscriptSlimCallSink = std::sync::Arc<std::sync::Mutex<Vec<TranscriptSlimCall>>>;

pub(super) trait PaneBindingOperations: Send + Sync {
    fn resolve(
        &self,
        input: &HookInput,
        is_session_start: bool,
        cached_has_parent_agent: Option<bool>,
    ) -> super::pane_binding::Resolution;
    fn set_session_id(&self, pane_id: &str, session_id: &str);
    fn current_socket_path(&self) -> anyhow::Result<Option<String>>;
}

struct LivePaneBindingOperations;

impl PaneBindingOperations for LivePaneBindingOperations {
    fn resolve(
        &self,
        input: &HookInput,
        is_session_start: bool,
        cached_has_parent_agent: Option<bool>,
    ) -> super::pane_binding::Resolution {
        super::pane_binding::resolve(input, is_session_start, cached_has_parent_agent)
    }

    fn set_session_id(&self, pane_id: &str, session_id: &str) {
        // Session metadata remains useful if tmux cannot persist the pane option.
        let _ = tmux::set_pane_option(pane_id, TMUX_SESSION_OPTION, session_id);
    }

    fn current_socket_path(&self) -> anyhow::Result<Option<String>> {
        Ok(tmux::current_socket_path()?)
    }
}

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
    /// Spawn the detached transcript slimmer after a large Claude compact.
    /// Off in tests (would fork a real process and survive past the test).
    pub(super) transcript_slim: bool,
    /// Spawn one detached session-title generation after an eligible Stop.
    pub(super) title_generation: bool,
    pub(super) capture_pr_context: Option<PrContextCapture>,
    /// Spawn the detached `a agent archive-tq-session-detached` worker on a
    /// genuine Ended transition. Off in tests (would fork a real process).
    pub(super) tq_archive: bool,
    /// Include `a agent bg run` state in Stop processing.
    pub(super) track_bg_run_tasks: bool,
    /// Keep SessionStart(resume) from waiting for message delivery readiness.
    pub(super) redeliver_bg_notifications: bool,
    pub(super) pane_binding: Option<Arc<dyn PaneBindingOperations>>,
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
    /// Test-only sink that records (session, transcript path) passed to the transcript slimmer.
    #[cfg(test)]
    pub(super) transcript_slim_calls: Option<TranscriptSlimCallSink>,
    /// Test-only sink that records detached title-generation requests.
    #[cfg(test)]
    pub(super) title_generation_calls: Option<TitleGenerationCallSink>,
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
impl SideEffects {
    pub(super) fn all() -> Self {
        Self {
            tmux: true,
            notifications: true,
            auto_compact: true,
            transcript_slim: true,
            title_generation: true,
            capture_pr_context: Some(Arc::new(super::super::session_pr_context::capture)),
            tq_archive: true,
            track_bg_run_tasks: true,
            redeliver_bg_notifications: true,
            pane_binding: Some(Arc::new(LivePaneBindingOperations)),
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
            transcript_slim_calls: None,
            #[cfg(test)]
            title_generation_calls: None,
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
            transcript_slim: false,
            title_generation: false,
            capture_pr_context: None,
            tq_archive: false,
            track_bg_run_tasks: false,
            redeliver_bg_notifications: false,
            pane_binding: None,
            removed_notification_groups: None,
            tmux_sync_calls: None,
            tq_archive_calls: None,
            notification_calls: None,
            auto_compact_calls: None,
            transcript_slim_calls: None,
            title_generation_calls: None,
            redelivery_calls: None,
        }
    }

    pub(super) fn resolve_pane(
        &self,
        input: &HookInput,
        is_session_start: bool,
        cached_has_parent_agent: Option<bool>,
    ) -> super::pane_binding::Resolution {
        self.pane_binding
            .as_ref()
            .map(|binding| binding.resolve(input, is_session_start, cached_has_parent_agent))
            .unwrap_or(super::pane_binding::Resolution {
                pane_info: None,
                has_parent_agent: cached_has_parent_agent,
            })
    }

    pub(super) fn set_pane_session_id(&self, pane_id: &str, session_id: &str) {
        if let Some(binding) = &self.pane_binding {
            binding.set_session_id(pane_id, session_id);
        }
    }

    pub(super) fn current_tmux_socket_path(&self) -> anyhow::Result<Option<String>> {
        self.pane_binding
            .as_ref()
            .map_or(Ok(None), |binding| binding.current_socket_path())
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

    pub(super) fn schedule_transcript_slim(&self, session_id: &str, transcript_path: &Path) {
        if !self.transcript_slim {
            return;
        }
        let args = transcript_slim_worker_args(session_id, transcript_path);
        #[cfg(test)]
        if let Some(calls) = &self.transcript_slim_calls {
            calls
                .lock()
                .expect("transcript_slim_calls mutex poisoned")
                .push((
                    session_id.to_string(),
                    transcript_path.to_path_buf(),
                    args.to_vec(),
                ));
        }
        #[cfg(not(test))]
        {
            let args = args.iter().map(String::as_str).collect::<Vec<_>>();
            crate::infra::process::spawn_self_detached(
                "agent.transcript.slim.spawn",
                "agent.transcript.slim.spawn_failed",
                session_id,
                &args,
            );
        }
    }

    pub(super) fn generate_title(
        &self,
        session_id: &str,
        prompt: &str,
        previous_label: Option<String>,
    ) -> bool {
        if !self.title_generation {
            return false;
        }
        let request = super::super::tui::title_generate::SpawnTitleGenerationRequest {
            session_id: session_id.to_string(),
            prompt: prompt.to_string(),
            previous_label,
            check_transcript_title: true,
        };
        #[cfg(test)]
        {
            if let Some(calls) = &self.title_generation_calls {
                calls
                    .lock()
                    .expect("title_generation_calls mutex poisoned")
                    .push((
                        request.session_id,
                        request.prompt,
                        request.previous_label,
                        request.check_transcript_title,
                    ));
                return true;
            }
            false
        }
        #[cfg(not(test))]
        {
            match super::super::tui::title_generate::spawn_detached_title_generation(request) {
                Ok(()) => true,
                Err(error) => {
                    tracing::warn!(
                        event = "agent.generate_title.spawn_failed",
                        session = %session_id,
                        error = %error,
                    );
                    false
                }
            }
        }
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

fn transcript_slim_worker_args(session_id: &str, transcript_path: &Path) -> [String; 6] {
    [
        "agent".to_string(),
        "transcript".to_string(),
        "slim".to_string(),
        session_id.to_string(),
        "--transcript-path".to_string(),
        transcript_path.to_string_lossy().into_owned(),
    ]
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use clap::Parser;

    use super::SideEffects;
    use crate::cli::{Cli, Commands};
    use crate::commands::agent::{AgentCommands, SlimArgs, TranscriptCommands};

    #[test]
    fn transcript_slim_worker_arguments_parse_as_the_hook_target() {
        let transcript_path = Path::new("/fixture/transcripts/fixture-session.jsonl");
        let calls = Arc::new(Mutex::new(Vec::new()));
        let side_effects = SideEffects {
            transcript_slim: true,
            transcript_slim_calls: Some(calls.clone()),
            ..SideEffects::none()
        };
        side_effects.schedule_transcript_slim("fixture-session", transcript_path);
        let calls = calls
            .lock()
            .expect("transcript slim calls should lock")
            .clone();
        let parsed_commands = calls
            .into_iter()
            .map(|(_, _, worker_args)| {
                let parsed = Cli::try_parse_from(
                    std::iter::once("a").chain(worker_args.iter().map(String::as_str)),
                )
                .expect("hook worker arguments should parse");
                parsed.command
            })
            .collect::<Vec<_>>();

        assert!(
            parsed_commands
                == vec![Commands::Agent(AgentCommands::Transcript(
                    TranscriptCommands::Slim(SlimArgs {
                        session_id: "fixture-session".to_string(),
                        cwd: PathBuf::from("."),
                        transcript_path: Some(transcript_path.to_path_buf()),
                    })
                ))]
        );
    }
}
