//! `a cc sweep` subcommand.
//!
//! Scans every session file on disk and pauses any session that has been
//! `Stopped` for longer than the configured timeout. Designed to be invoked
//! periodically (e.g., by a launchd agent on a 1-minute interval) rather than
//! spawned on demand from the Stop hook.
//!
//! Running sweep has no effect on sessions that are Running, WaitingInput,
//! Paused, or Ended -- the pure decision function `auto_pause::decide_pause`
//! owns the timeout policy; `PidResolver` owns the question of "which process
//! is hosting this session right now".

use std::fs;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use clap::{Args, Subcommand};

use super::auto_pause::{self, PauseDecision};
use super::pane_input;
use super::signal::{LibcSignalSender, SignalSender};
use super::store;
use super::types::{Session, SessionStatus};
use crate::infra::{process::ProcessSnapshot, tmux};
use crate::shared::config;

/// Pane option where we record a `<input-hash>,<unix_seconds>` snapshot
/// so the next sweep pass can tell whether the user has typed in the
/// Claude Code TUI input box since. We persist a hash, not the input
/// text itself, so the prompt content (which may include secrets) never
/// leaves the user's session.
const PANE_ACTIVITY_OPTION: &str = "@armyknife-pane-activity";

mod service;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct SweepArgs {
    #[command(subcommand)]
    pub command: Option<SweepCommands>,

    /// Override the timeout duration from config (for testing / manual runs).
    /// Accepts human-friendly durations like `30s`, `10m`, `1h30m`.
    /// Only used when running without a subcommand (i.e. a one-shot sweep).
    #[arg(long, global = true)]
    pub timeout: Option<String>,

    /// Dry-run: log what would be paused without sending signals or updating
    /// session files.
    #[arg(long, global = true)]
    pub dry_run: bool,
}

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum SweepCommands {
    /// Run a single sweep pass (default when no subcommand is given).
    Run,

    /// Install the launchd agent that runs sweep periodically (macOS).
    Install,

    /// Remove the launchd agent installed by `install`.
    Uninstall,

    /// Print the launchd agent status (plist path, bootstrapped state).
    Status,
}

/// Entry point for `a cc sweep`.
pub fn run(args: &SweepArgs) -> Result<()> {
    match args.command.clone().unwrap_or(SweepCommands::Run) {
        SweepCommands::Run => run_sweep(args),
        SweepCommands::Install => service::install(),
        SweepCommands::Uninstall => service::uninstall(),
        SweepCommands::Status => service::status(),
    }
}

fn run_sweep(args: &SweepArgs) -> Result<()> {
    let config = config::load_config().unwrap_or_default();

    // Respect the enabled flag unless a manual --timeout override was given.
    // (A manual `--timeout 1s` run is an explicit opt-in; we should honor it
    // even if the user set `enabled: false` in their config.)
    if args.timeout.is_none() && !config.cc.auto_pause.enabled {
        return Ok(());
    }

    let timeout_str = args
        .timeout
        .clone()
        .unwrap_or_else(|| config.cc.auto_pause.timeout.clone());
    let timeout = auto_pause::parse_duration(&timeout_str)
        .with_context(|| format!("invalid cc.auto_pause.timeout `{timeout_str}`"))?;

    let sessions_dir = store::sessions_dir()?;
    let sender = LibcSignalSender;
    let snapshot = ProcessSnapshot::capture();
    let probe = TmuxSessionProbe {
        snapshot: snapshot.as_ref(),
    };
    let report = sweep_impl(&sessions_dir, timeout, &sender, &probe, args.dry_run)?;

    // Always print a summary in --dry-run so the user can see the reasoning.
    // Otherwise only speak up when we actually paused something, since the
    // launchd-driven periodic invocation would otherwise flood the log.
    if report.paused > 0 || args.dry_run {
        eprintln!(
            "[armyknife] cc sweep: scanned={} paused={} waiting={} no_pid={} active={} (timeout={})",
            report.scanned,
            report.paused,
            report.waiting,
            report.no_pid,
            report.active,
            timeout_str,
        );
    }

    Ok(())
}

/// Abstraction over the tmux/process queries sweep needs at runtime.
///
/// Sweep runs detached from any claude process, so it cannot use its own
/// process ancestry. Instead it asks the probe two questions about each
/// candidate session:
///
/// - **When did this pane last show signs of activity?**
///   Returned by `last_activity_at`. We max this against `session.updated_at`
///   so we never pause a Stopped session whose user is still composing a
///   prompt in the input box.
/// - **Which pid should we SIGTERM?**
///   Returned by `resolve_pid`. Implemented by looking up the pane's
///   pane_pid and walking its descendants for a `claude` process.
pub(crate) trait SessionProbe {
    /// Returns the pid of the live `claude` process hosting `session`, if one
    /// can be located via the session's tmux pane.
    fn resolve_pid(&self, session: &Session) -> Option<u32>;

    /// Returns the timestamp at which we last observed the pane's input
    /// box text change, or `None` if unavailable. Implementations may
    /// persist the prior reading and compare against the live one to
    /// produce this value; callers must not depend on side effects.
    fn last_activity_at(&self, session: &Session, now: DateTime<Utc>) -> Option<DateTime<Utc>>;
}

struct TmuxSessionProbe<'a> {
    snapshot: Option<&'a ProcessSnapshot>,
}

/// Descendant walks are bounded to this many processes. A shell hosting
/// claude has at most a handful of children, so this is a safety cap rather
/// than an expected limit.
const MAX_DESCENDANT_NODES: usize = 64;

impl SessionProbe for TmuxSessionProbe<'_> {
    fn resolve_pid(&self, session: &Session) -> Option<u32> {
        let pane_id = &session.tmux_info.as_ref()?.pane_id;
        let pane_pid = tmux::get_pane_pid(pane_id)?;
        // When claude is launched directly as the pane command (no shell
        // wrapper), pane_pid itself is the claude process -- BFS from
        // pane_pid (exclusive) would miss it. Use the inclusive variant so
        // both "pane_pid is claude" and "pane_pid is zsh, child is claude"
        // resolve correctly.
        self.snapshot?
            .find_self_or_descendant_by_command(pane_pid, "claude", MAX_DESCENDANT_NODES)
    }

    fn last_activity_at(&self, session: &Session, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let pane_id = &session.tmux_info.as_ref()?.pane_id;
        let live = pane_input::get_pane_input_text(pane_id)?;
        let live_hash = hash_input_text(&live);
        let prior = tmux::get_pane_option(pane_id, PANE_ACTIVITY_OPTION)
            .as_deref()
            .and_then(parse_pane_activity);

        let observed_at = match prior {
            Some((prior_hash, ts)) if prior_hash == live_hash => ts,
            _ => now,
        };

        // Persist the live hash so the next sweep pass can detect a
        // change against it. Errors are non-fatal: a missed write just
        // means the next pass treats it as "first observation".
        let _ = tmux::set_pane_option(
            pane_id,
            PANE_ACTIVITY_OPTION,
            &format_pane_activity(live_hash, observed_at),
        );

        Some(observed_at)
    }
}

fn hash_input_text(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

fn parse_pane_activity(raw: &str) -> Option<(u64, DateTime<Utc>)> {
    let (hash_str, ts_str) = raw.trim().split_once(',')?;
    let hash: u64 = hash_str.trim().parse().ok()?;
    let ts: i64 = ts_str.trim().parse().ok()?;
    let observed_at = Utc.timestamp_opt(ts, 0).single()?;
    Some((hash, observed_at))
}

fn format_pane_activity(hash: u64, observed_at: DateTime<Utc>) -> String {
    format!("{},{}", hash, observed_at.timestamp())
}

/// Result of a single sweep pass. Exposed for tests and for the CLI summary.
///
/// The counters break down every non-ended session file that sweep considered
/// so users can see *why* a given session was or wasn't paused. Ended
/// sessions are not counted at all -- they match `a cc list`'s view of the
/// store.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SweepReport {
    /// Number of non-ended session files scanned.
    pub scanned: usize,
    /// Sessions that were (or would have been, in --dry-run) paused.
    pub paused: usize,
    /// Stopped sessions whose timeout has not yet elapsed.
    pub waiting: usize,
    /// Stopped sessions for which sweep could not locate a live `claude`
    /// process. Usually means the session's tmux pane is gone or no longer
    /// hosts claude (e.g., user exited with Ctrl-C without triggering
    /// SessionEnd).
    pub no_pid: usize,
    /// Sessions in an active state (Running, WaitingInput, Paused).
    pub active: usize,
}

/// Testable core of `run`. Reads every `*.json` session file under
/// `sessions_dir`, evaluates `decide_pause`, and pauses sessions whose
/// timeout has elapsed. Ended sessions are ignored entirely so that the
/// counts match what `a cc list` displays.
pub(crate) fn sweep_impl<S: SignalSender, P: SessionProbe>(
    sessions_dir: &Path,
    timeout: Duration,
    sender: &S,
    probe: &P,
    dry_run: bool,
) -> Result<SweepReport> {
    let mut report = SweepReport::default();

    if !sessions_dir.exists() {
        return Ok(report);
    }

    let now = Utc::now();

    // Read directory entries up-front so we don't hold the iterator while
    // mutating files inside the loop.
    let entries: Vec<_> = fs::read_dir(sessions_dir)
        .with_context(|| format!("reading sessions dir {}", sessions_dir.display()))?
        .filter_map(|e| e.ok())
        .collect();

    for entry in entries {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }

        // Extract session_id from the file stem so we can go through the
        // store helpers (which handle locking) instead of reading raw json.
        let Some(session_id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };

        let session = match store::load_session_from(sessions_dir, session_id)? {
            Some(s) => s,
            None => continue,
        };

        // Skip ended sessions so the scanned count lines up with `a cc list`.
        // Ended sessions are retained on disk only so that `claude -c` can
        // restore their label / ancestor chain -- they never need pausing.
        if session.status == SessionStatus::Ended {
            continue;
        }

        report.scanned += 1;

        // Fold the pane's last observed cursor-movement time into the
        // effective "last touched" time so a user who's still typing into a
        // Stopped pane (or a claude that's still streaming output) doesn't
        // get paused mid-stream.
        // N.B. We intentionally do NOT mutate session.updated_at here --
        // the effective timestamp is only for the timeout decision, not for
        // persisting to disk.
        let effective = effective_updated_at(&session, probe, now);

        match auto_pause::decide_pause_with_effective(&session, now, timeout, effective) {
            PauseDecision::Pause => {
                // Resolve the live claude pid for this session. If we can't
                // find one, the session's host process is already gone and
                // there is nothing to SIGTERM -- still count it for
                // observability.
                let Some(pid) = probe.resolve_pid(&session) else {
                    report.no_pid += 1;
                    continue;
                };

                if dry_run {
                    eprintln!(
                        "[armyknife] cc sweep (dry-run): would pause {} (pid={pid})",
                        session.session_id,
                    );
                    report.paused += 1;
                    continue;
                }
                pause_session(sessions_dir, session, pid, sender)?;
                report.paused += 1;
            }
            PauseDecision::NotYetElapsed => {
                report.waiting += 1;
            }
            PauseDecision::NotStopped => {
                report.active += 1;
            }
        }
    }

    Ok(report)
}

/// Returns the later of `session.updated_at` and the pane's last observed
/// cursor-movement time. If the probe can't report an activity timestamp
/// (not in tmux, pane gone, etc.) we fall back to `session.updated_at`
/// alone.
fn effective_updated_at<P: SessionProbe>(
    session: &Session,
    probe: &P,
    now: DateTime<Utc>,
) -> DateTime<Utc> {
    match probe.last_activity_at(session, now) {
        Some(activity_at) if activity_at > session.updated_at => activity_at,
        _ => session.updated_at,
    }
}

/// Sends SIGTERM to `pid` and flips the session status to Paused.
fn pause_session<S: SignalSender>(
    sessions_dir: &Path,
    mut session: Session,
    pid: u32,
    sender: &S,
) -> Result<()> {
    // Best-effort SIGTERM. ESRCH (process already gone) is not fatal -- we
    // still want to flip the status so `cc resume` can restore the session.
    if let Err(e) = sender.send(pid, libc::SIGTERM)
        && e.raw_os_error() != Some(libc::ESRCH)
    {
        eprintln!(
            "[armyknife] cc sweep: failed to SIGTERM pid {pid} for session {}: {e}",
            session.session_id
        );
    }

    session.status = SessionStatus::Paused;
    store::save_session_to(sessions_dir, &session)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::path::PathBuf;

    use chrono::{DateTime, TimeDelta, TimeZone, Utc};
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::super::signal::test_support::RecordingSender;
    use super::super::store::save_session_to;
    use super::super::types::{Session, SessionStatus};
    use super::*;

    struct TestDir {
        #[expect(dead_code, reason = "kept alive until test ends")]
        temp: TempDir,
        path: PathBuf,
    }

    #[fixture]
    fn test_dir() -> TestDir {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().to_path_buf();
        TestDir { temp, path }
    }

    /// Test double: looks up pids and tmux activity timestamps by
    /// session_id from caller-populated maps, so tests can simulate
    /// "claude is alive" vs "pane is gone" and "user is typing" vs
    /// "pane is idle" without actually spawning processes or touching tmux.
    #[derive(Default)]
    struct FakeProbe {
        pids: RefCell<HashMap<String, u32>>,
        activity: RefCell<HashMap<String, DateTime<Utc>>>,
    }

    impl FakeProbe {
        fn with_pids(pairs: &[(&str, u32)]) -> Self {
            let r = Self::default();
            for (id, pid) in pairs {
                r.pids.borrow_mut().insert((*id).to_string(), *pid);
            }
            r
        }

        fn with_last_activity(mut self, pairs: &[(&str, DateTime<Utc>)]) -> Self {
            for (id, ts) in pairs {
                self.activity.get_mut().insert((*id).to_string(), *ts);
            }
            self
        }
    }

    impl SessionProbe for FakeProbe {
        fn resolve_pid(&self, session: &Session) -> Option<u32> {
            self.pids.borrow().get(&session.session_id).copied()
        }

        fn last_activity_at(
            &self,
            session: &Session,
            _now: DateTime<Utc>,
        ) -> Option<DateTime<Utc>> {
            self.activity.borrow().get(&session.session_id).copied()
        }
    }

    fn make_session(id: &str, status: SessionStatus, updated_at: DateTime<Utc>) -> Session {
        Session {
            session_id: id.to_string(),
            cwd: PathBuf::from("/tmp"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status,
            created_at: updated_at,
            updated_at,
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            last_bg_task_pending: false,
        }
    }

    #[rstest]
    fn pauses_elapsed_stopped_session(test_dir: TestDir) {
        let old = Utc::now() - TimeDelta::hours(1);
        let session = make_session("sess-a", SessionStatus::Stopped, old);
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        let probe = FakeProbe::with_pids(&[("sess-a", 4242)]);
        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(1),
            &sender,
            &probe,
            false,
        )
        .expect("sweep");

        assert_eq!(report.scanned, 1);
        assert_eq!(report.paused, 1);
        assert_eq!(report.waiting, 0);
        assert_eq!(report.no_pid, 0);
        assert_eq!(report.active, 0);
        assert_eq!(*sender.calls.borrow(), vec![(4242, libc::SIGTERM)]);

        let reloaded = store::load_session_from(&test_dir.path, "sess-a")
            .expect("load")
            .expect("session exists");
        assert_eq!(reloaded.status, SessionStatus::Paused);
    }

    #[rstest]
    #[case::running(SessionStatus::Running)]
    #[case::waiting(SessionStatus::WaitingInput)]
    #[case::already_paused(SessionStatus::Paused)]
    fn counts_active_sessions(test_dir: TestDir, #[case] status: SessionStatus) {
        let old = Utc::now() - TimeDelta::hours(1);
        let session = make_session("sess-b", status, old);
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        let probe = FakeProbe::default();
        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(1),
            &sender,
            &probe,
            false,
        )
        .expect("sweep");

        assert_eq!(report.scanned, 1);
        assert_eq!(report.paused, 0);
        assert_eq!(report.active, 1);
        assert!(sender.calls.borrow().is_empty());

        let reloaded = store::load_session_from(&test_dir.path, "sess-b")
            .expect("load")
            .expect("session exists");
        assert_eq!(reloaded.status, status);
    }

    #[rstest]
    fn ended_sessions_are_not_scanned(test_dir: TestDir) {
        let old = Utc::now() - TimeDelta::hours(1);
        let session = make_session("sess-ended", SessionStatus::Ended, old);
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        let probe = FakeProbe::default();
        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(1),
            &sender,
            &probe,
            false,
        )
        .expect("sweep");

        // Ended sessions are skipped entirely so the scanned count matches
        // what `a cc list` displays.
        assert_eq!(report, SweepReport::default());
        assert!(sender.calls.borrow().is_empty());
    }

    #[rstest]
    fn counts_stopped_session_that_has_not_elapsed(test_dir: TestDir) {
        let recent = Utc::now();
        let session = make_session("sess-c", SessionStatus::Stopped, recent);
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        let probe = FakeProbe::with_pids(&[("sess-c", 4242)]);
        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(3600),
            &sender,
            &probe,
            false,
        )
        .expect("sweep");

        assert_eq!(report.scanned, 1);
        assert_eq!(report.paused, 0);
        assert_eq!(report.waiting, 1);
        assert!(sender.calls.borrow().is_empty());

        let reloaded = store::load_session_from(&test_dir.path, "sess-c")
            .expect("load")
            .expect("session exists");
        assert_eq!(reloaded.status, SessionStatus::Stopped);
    }

    #[rstest]
    fn counts_stopped_session_with_no_resolvable_pid(test_dir: TestDir) {
        let old = Utc::now() - TimeDelta::hours(1);
        let session = make_session("sess-no-pid", SessionStatus::Stopped, old);
        save_session_to(&test_dir.path, &session).expect("save");

        // Resolver returns None -- simulates a tmux pane that no longer
        // hosts claude (or a session without tmux_info at all).
        let sender = RecordingSender::default();
        let probe = FakeProbe::default();
        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(1),
            &sender,
            &probe,
            false,
        )
        .expect("sweep");

        assert_eq!(report.scanned, 1);
        assert_eq!(report.paused, 0);
        assert_eq!(report.no_pid, 1);
        assert!(sender.calls.borrow().is_empty());

        let reloaded = store::load_session_from(&test_dir.path, "sess-no-pid")
            .expect("load")
            .expect("session exists");
        assert_eq!(reloaded.status, SessionStatus::Stopped);
    }

    #[rstest]
    fn esrch_still_marks_paused(test_dir: TestDir) {
        let old = Utc::now() - TimeDelta::hours(1);
        let session = make_session("sess-d", SessionStatus::Stopped, old);
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        sender.fail_with_esrch.replace(true);
        let probe = FakeProbe::with_pids(&[("sess-d", 4242)]);
        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(1),
            &sender,
            &probe,
            false,
        )
        .expect("sweep");

        assert_eq!(report.paused, 1);
        assert_eq!(*sender.calls.borrow(), vec![(4242, libc::SIGTERM)]);

        let reloaded = store::load_session_from(&test_dir.path, "sess-d")
            .expect("load")
            .expect("session exists");
        assert_eq!(reloaded.status, SessionStatus::Paused);
    }

    #[rstest]
    fn recent_tmux_activity_blocks_pause(test_dir: TestDir) {
        // Session was marked Stopped an hour ago, so the naive timeout
        // check would pause it. But the pane's input box text changed a
        // few seconds ago -- the user is composing a follow-up prompt --
        // and must not be killed.
        let old = Utc::now() - TimeDelta::hours(1);
        let session = make_session("typing", SessionStatus::Stopped, old);
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        let recent_activity = Utc::now() - TimeDelta::seconds(5);
        let probe = FakeProbe::with_pids(&[("typing", 4242)])
            .with_last_activity(&[("typing", recent_activity)]);

        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(60),
            &sender,
            &probe,
            false,
        )
        .expect("sweep");

        assert_eq!(report.paused, 0);
        assert_eq!(report.waiting, 1);
        assert!(
            sender.calls.borrow().is_empty(),
            "active user must not be SIGTERM'd"
        );

        let reloaded = store::load_session_from(&test_dir.path, "typing")
            .expect("load")
            .expect("session exists");
        assert_eq!(reloaded.status, SessionStatus::Stopped);
    }

    #[rstest]
    fn tmux_activity_is_not_persisted_as_updated_at(test_dir: TestDir) {
        // Even when tmux pane activity extends the effective timeout, the
        // persisted updated_at must remain unchanged (the tmux timestamp is
        // only for the decision, not for the on-disk record).
        let old = Utc::now() - TimeDelta::hours(2);
        let session = make_session("persist-check", SessionStatus::Stopped, old);
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        // Tmux activity is old enough that the session still gets paused.
        let stale_activity = Utc::now() - TimeDelta::hours(1);
        let probe = FakeProbe::with_pids(&[("persist-check", 4242)])
            .with_last_activity(&[("persist-check", stale_activity)]);

        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(30 * 60),
            &sender,
            &probe,
            false,
        )
        .expect("sweep");

        assert_eq!(report.paused, 1);

        let reloaded = store::load_session_from(&test_dir.path, "persist-check")
            .expect("load")
            .expect("session exists");
        assert_eq!(reloaded.status, SessionStatus::Paused);
        // The persisted updated_at must be the original session time, not
        // the tmux pane activity timestamp.
        assert_eq!(
            reloaded.updated_at, old,
            "updated_at must not be overwritten with tmux activity timestamp"
        );
    }

    #[rstest]
    fn stale_tmux_activity_does_not_block_pause(test_dir: TestDir) {
        // Session updated an hour ago AND the window has been idle for
        // longer than the timeout. Normal pause path.
        let old = Utc::now() - TimeDelta::hours(1);
        let session = make_session("idle", SessionStatus::Stopped, old);
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        let stale_activity = Utc::now() - TimeDelta::minutes(45);
        let probe =
            FakeProbe::with_pids(&[("idle", 4242)]).with_last_activity(&[("idle", stale_activity)]);

        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(30 * 60),
            &sender,
            &probe,
            false,
        )
        .expect("sweep");

        assert_eq!(report.paused, 1);
        assert_eq!(*sender.calls.borrow(), vec![(4242, libc::SIGTERM)]);
    }

    #[rstest]
    fn dry_run_does_not_signal_or_save(test_dir: TestDir) {
        let old = Utc::now() - TimeDelta::hours(1);
        let session = make_session("sess-dry", SessionStatus::Stopped, old);
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        let probe = FakeProbe::with_pids(&[("sess-dry", 4242)]);
        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(1),
            &sender,
            &probe,
            true,
        )
        .expect("sweep");

        assert_eq!(report.paused, 1);
        assert!(
            sender.calls.borrow().is_empty(),
            "dry-run must not send signals"
        );

        let reloaded = store::load_session_from(&test_dir.path, "sess-dry")
            .expect("load")
            .expect("session exists");
        assert_eq!(
            reloaded.status,
            SessionStatus::Stopped,
            "dry-run must not update session status"
        );
    }

    #[rstest]
    fn handles_mixed_directory(test_dir: TestDir) {
        let now = Utc::now();
        let old = now - TimeDelta::hours(1);

        let pausable = make_session("pausable", SessionStatus::Stopped, old);
        let running = make_session("running", SessionStatus::Running, now);
        let recent_stop = make_session("recent", SessionStatus::Stopped, now);
        let no_pid = make_session("no-pid", SessionStatus::Stopped, old);
        let ended = make_session("ended", SessionStatus::Ended, old);

        for s in [&pausable, &running, &recent_stop, &no_pid, &ended] {
            save_session_to(&test_dir.path, s).expect("save");
        }

        let sender = RecordingSender::default();
        // Only `pausable` has a resolvable pid; `no-pid` is deliberately
        // absent from the map to simulate a dead pane.
        let probe = FakeProbe::with_pids(&[("pausable", 4242), ("running", 4243)]);
        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(60),
            &sender,
            &probe,
            false,
        )
        .expect("sweep");

        // Ended is filtered before counting; the remaining 4 break down into
        // one pause, one waiting (not elapsed), one no_pid, one active.
        assert_eq!(report.scanned, 4);
        assert_eq!(report.paused, 1);
        assert_eq!(report.waiting, 1);
        assert_eq!(report.no_pid, 1);
        assert_eq!(report.active, 1);
        assert_eq!(*sender.calls.borrow(), vec![(4242, libc::SIGTERM)]);
    }

    #[rstest]
    fn missing_sessions_dir_is_not_an_error(test_dir: TestDir) {
        let sender = RecordingSender::default();
        let probe = FakeProbe::default();
        let nonexistent = test_dir.path.join("does-not-exist");
        let report = sweep_impl(&nonexistent, Duration::from_secs(1), &sender, &probe, false)
            .expect("sweep should succeed even if dir is missing");
        assert_eq!(report, SweepReport::default());
    }

    #[rstest]
    fn pane_activity_round_trip() {
        // The pane option payload is hand-rolled CSV; round-tripping
        // through both helpers guards the format against drift on either
        // side.
        let observed_at = Utc.timestamp_opt(1_700_000_000, 0).single().expect("ts");
        let raw = format_pane_activity(0xdeadbeef_u64, observed_at);
        let parsed = parse_pane_activity(&raw).expect("parse");
        assert_eq!(parsed, (0xdeadbeef_u64, observed_at));
    }

    #[rstest]
    fn hash_input_text_distinguishes_typed_changes() {
        // Empty box vs typed prompt must collide-resist with overwhelming
        // probability for the activity probe to do its job.
        assert_ne!(hash_input_text(""), hash_input_text("hello"));
        assert_ne!(hash_input_text("draft v1"), hash_input_text("draft v2"));
    }

    #[rstest]
    fn hash_input_text_is_deterministic() {
        // The persisted hash is compared against a fresh hash on the
        // next sweep pass, so the function must return the same value
        // for the same input within a single binary build.
        assert_eq!(hash_input_text("hello"), hash_input_text("hello"));
    }

    #[rstest]
    #[case::empty("")]
    #[case::missing_ts("12345")]
    #[case::trailing_garbage("12345,1700000000,extra")]
    #[case::non_numeric_hash("abc,1700000000")]
    #[case::non_numeric_ts("12345,now")]
    fn parse_pane_activity_rejects_malformed(#[case] raw: &str) {
        assert!(parse_pane_activity(raw).is_none());
    }
}
