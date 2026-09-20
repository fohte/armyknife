//! `a agent peer wake` -- resume a paused Claude Code session from another
//! session's Bash tool, so its `SendMessage` name becomes resolvable.
//!
//! `a agent peer` can point at a session that `a agent sweep` has since paused:
//! its process has exited, so it has no entry in Claude Code's own session
//! registry (see `claude_registry`) and therefore no `SendMessage` name. `a
//! cc watch`'s TUI already knows how to respawn such a session's pane (see
//! `resume::respawn_paused_session`); this command drives the same respawn
//! from a non-interactive caller and waits for the new process to register
//! itself, so the caller gets back a name it can hand straight to
//! `SendMessage`. Unlike the TUI, it does not focus the pane (a resume
//! triggered from another session must not steal the user's tmux focus),
//! and it verifies the pane's last-known session ID matches the requested
//! one before respawning -- `a agent resume` (which the respawned pane runs)
//! resumes whatever session is recorded on the pane, not necessarily the
//! one this command was asked to wake.
//!
//! A Codex session has no `SendMessage` name to wait for, so waking one only
//! respawns the pane and returns no name. That is enough for
//! `peer::notify`: it first attempts app-server delivery, then falls back to
//! the persistent `codex queue` if the resumed thread is not registered yet.

use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use clap::Args;
use thiserror::Error;

use crate::commands::agent::claude_registry;
use crate::commands::agent::error::CcError;
use crate::commands::agent::resume::{RespawnError, respawn_paused_session};
use crate::commands::agent::store;
use crate::commands::agent::types::{Engine, Session, SessionStatus, TMUX_SESSION_OPTION};
use crate::infra::tmux;

/// How often to poll Claude Code's session registry for the resumed
/// process's name after respawning the pane.
const NAME_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// How long to wait for the resumed process to register its name before
/// giving up.
const NAME_POLL_TIMEOUT: Duration = Duration::from_secs(20);

/// Pane option holding the unix time (seconds) a wake last respawned the
/// pane. The pane keeps reading as an idle shell until the agent's process
/// takes it over (it first runs an interactive login shell, rc files and
/// all, before `a agent resume` execs the agent), so `respawn_paused_session`'s
/// idle check alone cannot tell a pane that was just respawned from one
/// that never was.
const RESPAWNED_AT_OPTION: &str = "@armyknife-wake-respawned-at";

/// How long after a respawn the pane is presumed to still be starting the
/// agent. Matches [`NAME_POLL_TIMEOUT`]: past that, a wake that respawned
/// the pane has given up on it.
const STARTUP_GRACE: Duration = NAME_POLL_TIMEOUT;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct WakeArgs {
    /// Session ID to wake -- the `session_id` from `a agent peer`
    pub session_id: String,
}

/// Runs the wake command: prints the resolved `SendMessage` name to stdout
/// (nothing for a Codex session, which has none).
pub fn run(args: &WakeArgs) -> Result<()> {
    if let Some(name) = wake(&args.session_id)? {
        println!("{name}");
    }
    Ok(())
}

/// Resumes `session_id` if paused and returns its resolved `SendMessage`
/// name, or `None` for a Codex session. `pub(super)` so `peer::notify` can
/// drive the same resume flow before delivering a message to a paused
/// session.
pub(super) fn wake(session_id: &str) -> Result<Option<String>> {
    wake_with(&System, session_id)
}

/// The one place the engines differ in how a session is woken. [`wake_with`]
/// is the shared flow (load -> pane check -> lock -> respawn); each hook
/// below is where that flow asks "what does this engine do here?".
trait EnginePolicy {
    /// Result for a session that is not `Paused`, so there is nothing to
    /// respawn.
    fn not_paused(&self, host: &dyn Host, session: &Session) -> Result<Option<String>>;

    /// Checked under the lock, before respawning: the name of a session that
    /// is already awake, if the engine has a way to tell.
    fn already_awake(&self, host: &dyn Host, session_id: &str) -> Option<String>;

    /// After the respawn, outside the lock: what to wait for, and what to
    /// hand back to the caller.
    fn await_awake(&self, host: &dyn Host, session_id: &str) -> Result<Option<String>>;
}

fn policy_for(engine: Engine) -> &'static dyn EnginePolicy {
    match engine {
        Engine::Claude => &ClaudePolicy,
        Engine::Codex => &CodexPolicy,
    }
}

struct ClaudePolicy;

impl EnginePolicy for ClaudePolicy {
    fn not_paused(&self, host: &dyn Host, session: &Session) -> Result<Option<String>> {
        host.registered_name(&session.session_id)
            .map(Some)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "No SendMessage name available for session {} (status: {})",
                    session.session_id,
                    session.status.display_name()
                )
            })
    }

    fn already_awake(&self, host: &dyn Host, session_id: &str) -> Option<String> {
        host.registered_name(session_id)
    }

    /// An unrelated process that happens to be named `claude` in the pane
    /// (see `respawn_unless_awake`) makes this time out rather than
    /// silently succeed.
    fn await_awake(&self, host: &dyn Host, session_id: &str) -> Result<Option<String>> {
        wait_for_name(host, session_id).map(Some)
    }
}

struct CodexPolicy;

impl EnginePolicy for CodexPolicy {
    /// No registry to check liveness against, so only the status armyknife
    /// itself tracks can be refused.
    fn not_paused(&self, _host: &dyn Host, session: &Session) -> Result<Option<String>> {
        if session.status == SessionStatus::Ended {
            bail!("Session {} has ended; not waking it", session.session_id)
        }
        Ok(None)
    }

    /// No registry to consult, so there is no name to find. A duplicate wake
    /// is only caught by the pane-level checks in `respawn_unless_awake`.
    fn already_awake(&self, _host: &dyn Host, _session_id: &str) -> Option<String> {
        None
    }

    /// Nothing to wait for: there is no name to resolve, and the resumed
    /// `codex` picks up its queue on its own. A busy pane named `codex` is
    /// therefore trusted as-is.
    fn await_awake(&self, _host: &dyn Host, _session_id: &str) -> Result<Option<String>> {
        Ok(None)
    }
}

fn wake_with(host: &dyn Host, session_id: &str) -> Result<Option<String>> {
    let sessions_dir = host.sessions_dir()?;
    let session = store::load_session_from(&sessions_dir, session_id)?
        .ok_or_else(|| CcError::SessionNotFound(session_id.to_string()))?;
    let policy = policy_for(session.engine);

    if session.status != SessionStatus::Paused {
        return policy.not_paused(host, &session);
    }

    let tmux_info = session
        .tmux_info
        .as_ref()
        .ok_or_else(|| CcError::NoTmuxInfo(session_id.to_string()))?;
    let recorded = host.pane_option(&tmux_info.pane_id, TMUX_SESSION_OPTION);
    check_pane_matches_target(recorded.as_deref(), session_id)?;

    let pane_id = &tmux_info.pane_id;
    if let Some(name) = respawn_unless_awake(host, policy, &sessions_dir, &session, pane_id)? {
        return Ok(Some(name));
    }
    policy.await_awake(host, session_id)
}

/// Respawns the paused session's pane unless the session turns out to be
/// awake already. Returns the awake session's name in that case, so the
/// caller has nothing left to await.
///
/// Concurrent wakes of the same paused session are serialized -- e.g.
/// several delegated children reporting back to the same paused parent at
/// once (the scenario `a agent new`'s envelope steers callers into).
/// Without this, two callers can both observe the pane still idle and both
/// respawn it, the second one killing the first one's freshly started
/// agent. The lock is released as soon as the pane is respawned (the
/// respawned `a agent resume` needs the session store's lock itself), so a
/// wake that follows straight after is held back by [`RESPAWNED_AT_OPTION`]
/// rather than by the pane's state.
fn respawn_unless_awake(
    host: &dyn Host,
    policy: &dyn EnginePolicy,
    sessions_dir: &Path,
    session: &Session,
    pane_id: &str,
) -> Result<Option<String>> {
    let _lock = store::lock_session_for_update(sessions_dir, &session.session_id)?;
    if let Some(name) = policy.already_awake(host, &session.session_id) {
        return Ok(Some(name));
    }
    let respawned_at = host.pane_option(pane_id, RESPAWNED_AT_OPTION);
    if respawned_recently(respawned_at.as_deref(), host.now_unix_secs()) {
        return Ok(None);
    }
    match host.respawn(session) {
        Ok(_pane_id) => {
            // Best effort: the pane is already respawned, so failing to mark
            // it must not fail the wake.
            let _ = host.set_pane_option(
                pane_id,
                RESPAWNED_AT_OPTION,
                &host.now_unix_secs().to_string(),
            );
        }
        // The pane already moved past the shell prompt into the session's
        // agent itself -- another wake (racing just outside this lock) or
        // the user beat us to it. Fall through instead of erroring;
        // `EnginePolicy::await_awake` decides what that means per engine.
        Err(RespawnError::PaneBusy(cmd)) if cmd == session.engine.process_name() => {}
        Err(e) => return Err(e).context("failed to resume the session's tmux pane"),
    }
    Ok(None)
}

/// Whether [`RESPAWNED_AT_OPTION`]'s value is within [`STARTUP_GRACE`] of
/// `now`. An unparsable value, or one from the future (clock stepped back),
/// counts as not recent so a bad marker can never block a wake for good.
fn respawned_recently(respawned_at: Option<&str>, now: i64) -> bool {
    respawned_at
        .and_then(|s| s.parse::<i64>().ok())
        .and_then(|at| u64::try_from(now - at).ok())
        .is_some_and(|age| age < STARTUP_GRACE.as_secs())
}

/// The outside world `wake` acts on, so tests can drive the flow (in
/// particular a race between two wakes) without tmux or Claude Code.
trait Host {
    fn sessions_dir(&self) -> Result<PathBuf>;
    fn pane_option(&self, pane_id: &str, option: &str) -> Option<String>;
    fn set_pane_option(&self, pane_id: &str, option: &str, value: &str) -> Result<()>;
    fn respawn(&self, session: &Session) -> Result<String, RespawnError>;
    fn registered_name(&self, session_id: &str) -> Option<String>;
    fn now_unix_secs(&self) -> i64;
}

struct System;

impl Host for System {
    fn sessions_dir(&self) -> Result<PathBuf> {
        store::sessions_dir()
    }

    fn pane_option(&self, pane_id: &str, option: &str) -> Option<String> {
        tmux::get_pane_option(pane_id, option)
    }

    fn set_pane_option(&self, pane_id: &str, option: &str, value: &str) -> Result<()> {
        Ok(tmux::set_pane_option(pane_id, option, value)?)
    }

    fn respawn(&self, session: &Session) -> Result<String, RespawnError> {
        respawn_paused_session(session)
    }

    fn registered_name(&self, session_id: &str) -> Option<String> {
        claude_registry::load_name_map().remove(session_id)
    }

    fn now_unix_secs(&self) -> i64 {
        Utc::now().timestamp()
    }
}

/// Error from [`check_pane_matches_target`]: the pane's recorded session
/// doesn't match the session this command was asked to wake.
#[derive(Debug, Error, PartialEq, Eq)]
enum WakeError {
    #[error(
        "pane's recorded session ({recorded:?}) does not match the requested session ({target}); refusing to resume a different session"
    )]
    PaneSessionMismatch {
        recorded: Option<String>,
        target: String,
    },
}

fn check_pane_matches_target(recorded: Option<&str>, target: &str) -> Result<(), WakeError> {
    if recorded == Some(target) {
        Ok(())
    } else {
        Err(WakeError::PaneSessionMismatch {
            recorded: recorded.map(str::to_string),
            target: target.to_string(),
        })
    }
}

fn wait_for_name(host: &dyn Host, session_id: &str) -> Result<String> {
    let deadline = Instant::now() + NAME_POLL_TIMEOUT;
    loop {
        if let Some(name) = host.registered_name(session_id) {
            return Ok(name);
        }
        if Instant::now() >= deadline {
            bail!("Timed out waiting for session {session_id} to reappear after resume");
        }
        thread::sleep(NAME_POLL_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

    use chrono::Utc;
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::*;
    use crate::commands::agent::types::TmuxInfo;

    const SESSION_ID: &str = "s1";
    const PANE_ID: &str = "%0";
    const NAME: &str = "agent-1";

    #[rstest]
    #[case::matches(Some("abc"), "abc", Ok(()))]
    #[case::mismatched(
        Some("xyz"),
        "abc",
        Err(WakeError::PaneSessionMismatch {
            recorded: Some("xyz".to_string()),
            target: "abc".to_string(),
        })
    )]
    #[case::unset(
        None,
        "abc",
        Err(WakeError::PaneSessionMismatch {
            recorded: None,
            target: "abc".to_string(),
        })
    )]
    fn check_pane_matches_target_cases(
        #[case] recorded: Option<&str>,
        #[case] target: &str,
        #[case] expected: std::result::Result<(), WakeError>,
    ) {
        assert_eq!(check_pane_matches_target(recorded, target), expected);
    }

    fn session(engine: Engine, status: SessionStatus) -> Session {
        Session {
            session_id: SESSION_ID.to_string(),
            cwd: PathBuf::from("/tmp/test"),
            transcript_path: None,
            tty: None,
            tmux_info: Some(TmuxInfo {
                session_name: "main".to_string(),
                window_name: "editor".to_string(),
                window_index: 0,
                pane_id: PANE_ID.to_string(),
            }),
            status,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: Default::default(),
            pending_agent_task_ids: Default::default(),
            pending_permission_agent_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            engine,
        }
    }

    /// In-memory stand-in for tmux and Claude Code's registry, backed by a
    /// real (temporary) session store so the wake lock is the real one.
    ///
    /// `respawn` mirrors `resume::respawn_paused_session`: it refuses a pane
    /// that is not at a shell prompt, and the check and the respawn are
    /// separate steps (two tmux round-trips in the real one), which is the
    /// window the wake lock exists to close.
    struct FakeHost {
        dir: TempDir,
        pane_cmd: Mutex<&'static str>,
        pane_options: Mutex<HashMap<String, String>>,
        registered: Mutex<Option<String>>,
        respawns: AtomicUsize,
        now: AtomicI64,
        /// Whether the agent is up as soon as the pane is respawned. A real
        /// respawn is not: the pane runs an interactive login shell (rc
        /// files and all) before `a agent resume` execs the agent, and
        /// until then `pane_current_command` still reads as a shell prompt.
        agent_up_on_respawn: bool,
    }

    impl FakeHost {
        fn new(engine: Engine, status: SessionStatus) -> Self {
            let dir = TempDir::new().expect("temp dir creation should succeed");
            store::save_session_to(dir.path(), &session(engine, status))
                .expect("saving the session should succeed");
            Self {
                agent_up_on_respawn: true,
                dir,
                pane_cmd: Mutex::new("zsh"),
                pane_options: Mutex::new(HashMap::from([(
                    TMUX_SESSION_OPTION.to_string(),
                    SESSION_ID.to_string(),
                )])),
                registered: Mutex::new(None),
                respawns: AtomicUsize::new(0),
                now: AtomicI64::new(1_000_000),
            }
        }

        fn slow_startup(mut self) -> Self {
            self.agent_up_on_respawn = false;
            self
        }

        fn with_pane_cmd(self, cmd: &'static str) -> Self {
            *self.pane_cmd.lock().expect("pane lock") = cmd;
            self
        }

        fn advance_clock(&self, secs: i64) {
            self.now.fetch_add(secs, Ordering::SeqCst);
        }

        fn respawns(&self) -> usize {
            self.respawns.load(Ordering::SeqCst)
        }
    }

    impl Host for FakeHost {
        fn sessions_dir(&self) -> Result<PathBuf> {
            Ok(self.dir.path().to_path_buf())
        }

        fn pane_option(&self, _pane_id: &str, option: &str) -> Option<String> {
            self.pane_options.lock().ok()?.get(option).cloned()
        }

        fn set_pane_option(&self, _pane_id: &str, option: &str, value: &str) -> Result<()> {
            self.pane_options
                .lock()
                .expect("pane options lock")
                .insert(option.to_string(), value.to_string());
            Ok(())
        }

        fn respawn(&self, session: &Session) -> Result<String, RespawnError> {
            let cmd = *self.pane_cmd.lock().expect("pane lock");
            if cmd != "zsh" {
                return Err(RespawnError::PaneBusy(cmd.to_string()));
            }
            std::thread::sleep(Duration::from_millis(20));
            self.respawns.fetch_add(1, Ordering::SeqCst);
            if self.agent_up_on_respawn {
                *self.pane_cmd.lock().expect("pane lock") = session.engine.process_name();
                *self.registered.lock().expect("registry lock") = Some(NAME.to_string());
            }
            Ok(PANE_ID.to_string())
        }

        fn registered_name(&self, _session_id: &str) -> Option<String> {
            self.registered.lock().ok()?.clone()
        }

        fn now_unix_secs(&self) -> i64 {
            self.now.load(Ordering::SeqCst)
        }
    }

    #[fixture]
    fn claude_paused() -> FakeHost {
        FakeHost::new(Engine::Claude, SessionStatus::Paused)
    }

    #[fixture]
    fn codex_paused() -> FakeHost {
        FakeHost::new(Engine::Codex, SessionStatus::Paused)
    }

    fn wake_result(host: &FakeHost) -> std::result::Result<Option<String>, String> {
        wake_with(host, SESSION_ID).map_err(|e| e.to_string())
    }

    /// The locked respawn step of one wake, without the wait that follows it
    /// -- a wake of a Claude session whose agent never registers would
    /// otherwise block until `NAME_POLL_TIMEOUT`.
    fn respawn_once(
        host: &FakeHost,
        engine: Engine,
    ) -> std::result::Result<Option<String>, String> {
        let session = session(engine, SessionStatus::Paused);
        respawn_unless_awake(host, policy_for(engine), host.dir.path(), &session, PANE_ID)
            .map_err(|e| e.to_string())
    }

    #[rstest]
    #[case::claude_registered(
        Engine::Claude,
        SessionStatus::Stopped,
        Some(NAME),
        Ok(Some(NAME.to_string()))
    )]
    #[case::claude_unregistered(
        Engine::Claude,
        SessionStatus::Stopped,
        None,
        Err("No SendMessage name available for session s1 (status: stopped)".to_string())
    )]
    #[case::claude_ended(
        Engine::Claude,
        SessionStatus::Ended,
        None,
        Err("No SendMessage name available for session s1 (status: ended)".to_string())
    )]
    #[case::codex_running(Engine::Codex, SessionStatus::Running, None, Ok(None))]
    #[case::codex_stopped(Engine::Codex, SessionStatus::Stopped, None, Ok(None))]
    #[case::codex_ended(
        Engine::Codex,
        SessionStatus::Ended,
        None,
        Err("Session s1 has ended; not waking it".to_string())
    )]
    fn not_paused_never_respawns(
        #[case] engine: Engine,
        #[case] status: SessionStatus,
        #[case] registered: Option<&str>,
        #[case] expected: std::result::Result<Option<String>, String>,
    ) {
        let host = FakeHost::new(engine, status);
        *host.registered.lock().expect("registry lock") = registered.map(str::to_string);

        assert_eq!(
            (wake_result(&host), host.respawns()),
            (expected, 0),
            "engine={engine:?} status={status:?}"
        );
    }

    #[rstest]
    #[case::claude(claude_paused(), Some(NAME.to_string()))]
    #[case::codex(codex_paused(), None)]
    fn concurrent_wakes_respawn_once(#[case] host: FakeHost, #[case] expected: Option<String>) {
        let results: Vec<_> = thread::scope(|scope| {
            let handles: Vec<_> = (0..2).map(|_| scope.spawn(|| wake_result(&host))).collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("wake thread should not panic"))
                .collect()
        });

        assert_eq!(
            (results, host.respawns()),
            (vec![Ok(expected.clone()), Ok(expected)], 1)
        );
    }

    // The second wake takes the lock as soon as the first releases it,
    // while the first one's agent is still starting and the pane still
    // reads as an idle shell.
    #[rstest]
    #[case::claude(Engine::Claude)]
    #[case::codex(Engine::Codex)]
    fn wake_right_after_a_respawn_does_not_respawn_again(#[case] engine: Engine) {
        let host = FakeHost::new(engine, SessionStatus::Paused).slow_startup();

        let results = [respawn_once(&host, engine), respawn_once(&host, engine)];

        assert_eq!((results, host.respawns()), ([Ok(None), Ok(None)], 1));
    }

    // A respawn whose agent never came up (e.g. `claude` not on PATH) must
    // not lock the session out of being woken again.
    #[rstest]
    #[case::claude(Engine::Claude)]
    #[case::codex(Engine::Codex)]
    fn wake_after_the_startup_grace_respawns_again(#[case] engine: Engine) {
        let host = FakeHost::new(engine, SessionStatus::Paused).slow_startup();

        let first = respawn_once(&host, engine);
        host.advance_clock(i64::try_from(STARTUP_GRACE.as_secs()).expect("grace fits in i64"));
        let second = respawn_once(&host, engine);

        assert_eq!(
            ([first, second], host.respawns()),
            ([Ok(None), Ok(None)], 2)
        );
    }

    #[rstest]
    #[case::claude_own_agent(Engine::Claude, "claude", Ok(None), 0)]
    #[case::codex_own_agent(Engine::Codex, "codex", Ok(None), 0)]
    #[case::claude_other_program(
        Engine::Claude,
        "nvim",
        Err("failed to resume the session's tmux pane".to_string()),
        0
    )]
    #[case::codex_other_program(
        Engine::Codex,
        "nvim",
        Err("failed to resume the session's tmux pane".to_string()),
        0
    )]
    // Codex's process is not `claude`, and vice versa.
    #[case::claude_pane_runs_codex(
        Engine::Claude,
        "codex",
        Err("failed to resume the session's tmux pane".to_string()),
        0
    )]
    fn busy_pane_is_left_alone(
        #[case] engine: Engine,
        #[case] pane_cmd: &'static str,
        #[case] expected: std::result::Result<Option<String>, String>,
        #[case] expected_respawns: usize,
    ) {
        let host = FakeHost::new(engine, SessionStatus::Paused).with_pane_cmd(pane_cmd);

        assert_eq!(
            (respawn_once(&host, engine), host.respawns()),
            (expected, expected_respawns)
        );
    }

    #[rstest]
    #[case::unset(None, false)]
    #[case::not_a_number(Some("yesterday"), false)]
    #[case::just_now(Some("1000000"), true)]
    #[case::last_second_of_grace(Some("999981"), true)]
    #[case::grace_elapsed(Some("999980"), false)]
    #[case::from_the_future(Some("1000005"), false)]
    fn respawned_recently_cases(#[case] respawned_at: Option<&str>, #[case] expected: bool) {
        assert_eq!(respawned_recently(respawned_at, 1_000_000), expected);
    }
}
