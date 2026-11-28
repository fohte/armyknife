//! `a agent sweep` subcommand.
//!
//! Scans session records to mark terminal Codex turns, pause sessions that
//! have been `Stopped` for longer than the configured timeout, and reap live
//! processes whose records are already `Paused`. A first observation marks a
//! paused process as awaiting its resume hook; if the next sweep still sees
//! `Paused`, it requests shutdown. Designed to be
//! invoked periodically (e.g., by a launchd agent on a 1-minute interval)
//! rather than spawned on demand from the Stop hook.
//!
//! Claude Code (Node.js) can keep running for a long time after receiving
//! SIGTERM while blocked in known-flaky code paths (a stalled SSE stream, a
//! Bash-tool thread deadlock, or a synchronous CPU spin during context
//! compaction). Sending the signal is not proof the process has exited, so
//! its session stays `Stopped` until a later sweep can no longer resolve the
//! pid. Codex can instead be confirmed `Paused` in the same run when it exits
//! within the graceful Ctrl+D window. Remaining live processes receive
//! SIGTERM again on later runs with no retry limit.
//!
//! While a session waits in that signaled-but-not-yet-Paused window, it is
//! marked `Session::sweep_signaled`. Paused sessions use the same marker to
//! defer signaling for one sweep so a resume hook can update their status.
//! Codex gets one chance to exit through Ctrl+D so it can restore the terminal
//! before the same sweep falls back to SIGTERM. For a still-`Stopped` session,
//! the flag also lets the `SessionEnd` hook (see `hook.rs`) recognize a process
//! exiting in response to sweep's own shutdown request and confirm it as
//! `Paused` there and then, rather than misreading the status as the user
//! manually ending the session.
//!
//! Auto-pause only considers Stopped sessions. A separate pass closes idle
//! Stopped or Paused sessions after their linked-worktree pull request merges.
//! Another retries recent background completion notifications that could not be
//! delivered, waking the session when it is Paused.

use std::future::Future;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Args, Subcommand};

use super::auto_pause::{self, PauseDecision};
use super::graceful_quit::GracefulQuitRequester;
use super::signal::{LibcSignalSender, SignalSender};
use super::store;
use super::tmux_sync::{LiveTmuxStatusSyncer, TmuxStatusSyncer};
use super::types::{Engine, Session, SessionStatus};
use crate::infra::process::ProcessSnapshot;
use crate::shared::active_session::{TmuxDraftProbe, effective_updated_at_from_observation};
use crate::shared::config;
use crate::shared::log::short_run_id;

mod codex_turn_status;
mod crit_cleanup;
mod deferred_stop;
mod merged_close;
mod service;
mod session_probe;
mod undelivered_redelivery;

use session_probe::{SessionProbe, TmuxSessionProbe};

#[derive(Args, Clone, PartialEq, Eq)]
pub struct SweepArgs {
    #[command(subcommand)]
    pub command: Option<SweepCommands>,

    /// Override the timeout duration from config (for testing / manual runs).
    /// Accepts human-friendly durations like `30s`, `10m`, `1h30m`.
    /// Only used when running without a subcommand (i.e. a one-shot sweep).
    #[arg(long, global = true)]
    pub timeout: Option<String>,

    /// Dry-run: log what would be paused, marked failed or stopped, closed, or
    /// redelivered without sending signals or updating session records.
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

/// Entry point for `a agent sweep`.
pub async fn run(args: &SweepArgs) -> Result<()> {
    match args.command.clone().unwrap_or(SweepCommands::Run) {
        SweepCommands::Run => run_sweep(args).await,
        SweepCommands::Install => service::install(),
        SweepCommands::Uninstall => service::uninstall(),
        SweepCommands::Status => service::status(),
    }
}

async fn run_sweep(args: &SweepArgs) -> Result<()> {
    let run_id = short_run_id();
    let span = tracing::info_span!("agent.sweep", run_id = %run_id);
    let _entered = span.enter();

    let config = config::load_config_or_default();

    let pause_enabled = args.timeout.is_some() || config.agent.auto_pause.enabled;
    let timeout_str = args
        .timeout
        .clone()
        .unwrap_or_else(|| config.agent.auto_pause.timeout.clone());
    let sessions_dir = store::sessions_dir()?;
    tracing::info!(
        event = "agent.sweep.start",
        timeout = %timeout_str,
        dry_run = args.dry_run,
        pause_enabled,
    );
    let crit_links_cleared = recover_sweep_pass(
        crit_cleanup::clear_stale_links(&sessions_dir, args.dry_run),
        "agent.sweep.crit_cleanup_failed",
        "stale crit-link cleanup failed",
        &mut |warning| eprintln!("{warning}"),
    );
    let pause_pass = || -> Result<SweepReport> {
        let timeout = auto_pause::parse_duration(&timeout_str)
            .with_context(|| format!("invalid agent.auto_pause.timeout `{timeout_str}`"))?;
        let tasks_dir = super::bg_tasks::tasks_dir()?;
        let sender = LibcSignalSender;
        let snapshot = ProcessSnapshot::capture_with_command_lines();
        let probe = TmuxSessionProbe {
            snapshot: snapshot.as_ref(),
            drafts: TmuxDraftProbe::default(),
            tasks_dir,
        };
        let syncer = LiveTmuxStatusSyncer;
        sweep_impl(
            &sessions_dir,
            timeout,
            &sender,
            &probe,
            &syncer,
            args.dry_run,
        )
    };
    let codex_status_pass = || {
        codex_turn_status::mark_finished_codex_sessions(
            &sessions_dir,
            &LiveTmuxStatusSyncer,
            args.dry_run,
        )
    };
    let (report, codex_updates, merged_closed, redelivery_started) = run_sweep_passes(
        pause_enabled,
        pause_pass,
        codex_status_pass,
        || merged_close::close_merged_sessions(args.dry_run),
        || undelivered_redelivery::spawn_recent(&sessions_dir, args.dry_run),
        |warning| eprintln!("{warning}"),
    )
    .await;
    let deferred_stop_notifications = recover_sweep_pass(
        deferred_stop::notify_ready(
            &sessions_dir,
            config.agent.unreported_stop_notification.as_deref(),
            args.dry_run,
        ),
        "agent.sweep.deferred_unreported_stop_failed",
        "deferred unreported-stop notification failed",
        &mut |warning| eprintln!("{warning}"),
    );
    tracing::info!(
        event = "agent.sweep.summary",
        scanned = report.scanned,
        paused = report.paused,
        signaled = report.signaled,
        waiting = report.waiting,
        active = report.active,
        codex_failed = codex_updates.failed,
        codex_interrupted = codex_updates.interrupted,
        merged_closed,
        crit_links_cleared,
        redelivery_started,
        deferred_stop_notifications,
        timeout = %timeout_str,
        dry_run = args.dry_run,
    );

    // Always print a summary in --dry-run so the user can see the reasoning.
    // Otherwise only speak up when a pause, terminal Codex turn, or merged-session close succeeded.
    // A session stuck in the signaled-but-not-Paused window (see module docs) gets
    // re-signaled on every sweep pass with no retry limit, so gating on
    // `signaled` too would flood the log for exactly the stuck-process
    // scenario this module exists to handle; the structured `tracing::info!`
    // above already records every signal for anyone tailing the JSONL log.
    if report.paused > 0
        || codex_updates.total() > 0
        || merged_closed > 0
        || crit_links_cleared > 0
        || deferred_stop_notifications > 0
        || args.dry_run
    {
        eprintln!(
            "[armyknife] agent sweep: scanned={} paused={} codex_failed={} codex_interrupted={} merged_closed={} crit_links_cleared={} redelivery_started={} deferred_stop_notifications={} signaled={} waiting={} active={} (timeout={})",
            report.scanned,
            report.paused,
            codex_updates.failed,
            codex_updates.interrupted,
            merged_closed,
            crit_links_cleared,
            redelivery_started,
            deferred_stop_notifications,
            report.signaled,
            report.waiting,
            report.active,
            timeout_str,
        );
    }

    Ok(())
}

async fn run_sweep_passes<P, C, M, Fut, R, W>(
    pause_enabled: bool,
    pause_pass: P,
    codex_status_pass: C,
    merged_close_pass: M,
    redelivery_pass: R,
    mut on_pass_failure: W,
) -> (
    SweepReport,
    codex_turn_status::CodexTurnUpdates,
    usize,
    usize,
)
where
    P: FnOnce() -> Result<SweepReport>,
    C: FnOnce() -> Result<codex_turn_status::CodexTurnUpdates>,
    M: FnOnce() -> Fut,
    Fut: Future<Output = Result<usize>>,
    R: FnOnce() -> Result<usize>,
    W: FnMut(&str),
{
    let report = if pause_enabled {
        recover_sweep_pass(
            pause_pass(),
            "agent.sweep.auto_pause_failed",
            "auto-pause failed",
            &mut on_pass_failure,
        )
    } else {
        SweepReport::default()
    };
    let codex_updates = recover_sweep_pass(
        codex_status_pass(),
        "agent.sweep.codex_status_failed",
        "Codex status failed",
        &mut on_pass_failure,
    );
    let merged_closed = recover_sweep_pass(
        merged_close_pass().await,
        "agent.sweep.sessions_read_failed",
        "merged-session close failed",
        &mut on_pass_failure,
    );
    // Runs last so it sees the sessions the other passes signaled or closed.
    let redelivery_started = recover_sweep_pass(
        redelivery_pass(),
        "agent.sweep.redelivery_failed",
        "undelivered notification redelivery failed",
        &mut on_pass_failure,
    );
    (report, codex_updates, merged_closed, redelivery_started)
}

fn recover_sweep_pass<T, W>(
    result: Result<T>,
    event: &'static str,
    failure_label: &str,
    on_pass_failure: &mut W,
) -> T
where
    T: Default,
    W: FnMut(&str),
{
    match result {
        Ok(value) => value,
        Err(error) => {
            let error_chain = format!("{error:#}");
            tracing::warn!(event = event, error = %error_chain);
            let warning = format!("[armyknife] agent sweep: {failure_label}: {error_chain}");
            on_pass_failure(&warning);
            T::default()
        }
    }
}

/// Result of the pause pass. Exposed for tests and for the CLI summary.
///
/// These counters explain why each non-ended session was or was not paused.
/// Ended sessions are not counted, matching `a agent list`'s view of the
/// store. Codex failure detections are counted separately.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SweepReport {
    /// Number of non-ended session records scanned.
    pub scanned: usize,
    /// Sessions confirmed as `Paused` (or that would have been, in
    /// --dry-run) this run. This means a prior sweep's SIGTERM (or the
    /// process exiting on its own) has actually taken effect, including a
    /// Codex process that exits within this run's graceful shutdown window.
    pub paused: usize,
    /// Sessions that received a shutdown request this run, including paused
    /// records with live processes that have not exited yet.
    pub signaled: usize,
    /// Stopped sessions whose timeout has not yet elapsed.
    pub waiting: usize,
    /// Sessions that are active, have a detached background command or
    /// Remote Control connection, or whose process state is unresolved.
    pub active: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PausedReapPreparation {
    Armed,
    Ready,
    SessionChanged,
}

#[derive(Debug, PartialEq, Eq)]
enum ShutdownOutcome {
    Exited,
    StillRunning,
    SessionChanged,
}

/// Marks a paused process after resolving it. Waiting one sweep gives a resume
/// hook time to move the session out of `Paused` before shutdown is requested.
fn prepare_paused_reap(sessions_dir: &Path, session: &Session) -> Result<PausedReapPreparation> {
    let mut preparation = PausedReapPreparation::SessionChanged;
    store::update_session_in(sessions_dir, &session.session_id, |current| {
        if current.status != SessionStatus::Paused || current.updated_at != session.updated_at {
            return false;
        }
        if current.sweep_signaled {
            preparation = PausedReapPreparation::Ready;
            return false;
        }
        current.sweep_signaled = true;
        preparation = PausedReapPreparation::Armed;
        true
    })?;
    Ok(preparation)
}

fn clear_paused_reap_marker(sessions_dir: &Path, session: &Session) -> Result<()> {
    store::update_session_in(sessions_dir, &session.session_id, |current| {
        if current.status != SessionStatus::Paused
            || current.updated_at != session.updated_at
            || !current.sweep_signaled
        {
            return false;
        }
        current.sweep_signaled = false;
        true
    })
}

/// Signals a previously observed paused process only while its record still
/// describes the same paused session. Holding the session lock prevents a
/// resume hook from changing the status between the check and signal delivery.
fn signal_paused_process<S: SignalSender>(
    sessions_dir: &Path,
    session: &Session,
    pid: u32,
    sender: &S,
) -> Result<bool> {
    let lock = store::lock_session_for_update(sessions_dir, &session.session_id)?;
    let Some(current) = lock.load()? else {
        return Ok(false);
    };
    if current.status != SessionStatus::Paused
        || current.updated_at != session.updated_at
        || !current.sweep_signaled
    {
        return Ok(false);
    }

    if let Err(error) = sender.send(pid, libc::SIGTERM)
        && error.raw_os_error() != Some(libc::ESRCH)
    {
        tracing::warn!(
            event = "agent.sweep.sigterm_failed",
            session = %session.session_id,
            pid,
            error = %error,
        );
    }
    Ok(true)
}

/// Testable core of `run`. Reads sessions under `sessions_dir`, evaluates
/// `decide_pause`, and pauses sessions whose timeout has elapsed. Ended
/// sessions are ignored entirely so that the counts match `a agent list`.
fn sweep_impl<S, P, T>(
    sessions_dir: &Path,
    timeout: Duration,
    sender: &S,
    probe: &P,
    syncer: &T,
    dry_run: bool,
) -> Result<SweepReport>
where
    S: SignalSender + GracefulQuitRequester,
    P: SessionProbe,
    T: TmuxStatusSyncer,
{
    let mut report = SweepReport::default();

    let now = Utc::now();

    for listed_session in store::list_all_sessions_from(sessions_dir)? {
        let Some(mut session) = store::load_session_from(sessions_dir, &listed_session.session_id)?
        else {
            continue;
        };
        if session.archived_at.is_some() {
            continue;
        }

        // Skip ended sessions so the scanned count lines up with `a agent list`.
        // Ended sessions are retained on disk only so that `claude -c` can
        // restore their label / ancestor chain -- they never need pausing.
        if session.status == SessionStatus::Ended {
            continue;
        }

        report.scanned += 1;

        if probe.has_pending_bg_run(&session) {
            tracing::info!(
                event = "agent.sweep.bg_run_pending",
                session = %session.session_id,
            );
            report.active += 1;
            continue;
        }

        if session.status == SessionStatus::Paused {
            match probe.resolve_pids(&session) {
                Ok(None) => {
                    tracing::warn!(
                        event = "agent.sweep.process_unresolved",
                        session = %session.session_id,
                    );
                    report.active += 1;
                }
                Ok(Some(pids)) if pids.is_empty() => {
                    if !dry_run {
                        clear_paused_reap_marker(sessions_dir, &session)?;
                    }
                }
                Ok(Some(pids))
                    if pids
                        .iter()
                        .any(|pid| probe.has_remote_control(&session, *pid)) =>
                {
                    tracing::info!(
                        event = "agent.sweep.remote_control_active",
                        session = %session.session_id,
                    );
                    report.active += 1;
                }
                Ok(Some(pids)) => {
                    if dry_run && !session.sweep_signaled {
                        tracing::info!(
                            event = "agent.sweep.dry_run_paused_process_wait_for_hook",
                            session = %session.session_id,
                        );
                        report.active += 1;
                    } else if dry_run {
                        for pid in &pids {
                            tracing::info!(
                                event = "agent.sweep.dry_run_paused_process_signal",
                                session = %session.session_id,
                                pid,
                            );
                            eprintln!(
                                "[armyknife] agent sweep (dry-run): would signal paused {} (pid={pid})",
                                session.session_id,
                            );
                        }
                        report.signaled += 1;
                    } else {
                        match prepare_paused_reap(sessions_dir, &session)? {
                            PausedReapPreparation::Armed => {
                                tracing::info!(
                                    event = "agent.sweep.paused_process_waiting_for_hook",
                                    session = %session.session_id,
                                );
                                report.active += 1;
                            }
                            PausedReapPreparation::Ready => {
                                let mut signaled = false;
                                for pid in pids {
                                    if signal_paused_process(sessions_dir, &session, pid, sender)? {
                                        tracing::info!(
                                            event = "agent.sweep.paused_process_signaled",
                                            session = %session.session_id,
                                            pid,
                                        );
                                        signaled = true;
                                    } else {
                                        break;
                                    }
                                }
                                if signaled {
                                    report.signaled += 1;
                                }
                            }
                            PausedReapPreparation::SessionChanged => {}
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        event = "agent.sweep.process_query_failed",
                        session = %session.session_id,
                        error = %error,
                    );
                    report.active += 1;
                }
            }
            continue;
        }

        // `pending_bg_task_ids` / `pending_agent_task_ids` are only ever
        // refreshed by a future `Stop` hook (see `hook.rs`). If the `claude`
        // process backing this session crashed or was killed outside of
        // sweep's own SIGTERM path, no further hook will ever fire for it,
        // so those sets would otherwise stay non-empty forever and
        // permanently block both auto-pause here and `agent clean`'s worktree
        // protection. Once sweep can independently confirm no `claude` pid
        // resolves for the session, treat any pending ids as stale and drop
        // them before making the pause decision.
        if session.has_pending_bg_tasks() {
            match probe.resolve_pids(&session) {
                Ok(Some(pids)) if pids.is_empty() => {
                    tracing::info!(
                        event = "agent.sweep.stale_pending_tasks_cleared",
                        session = %session.session_id,
                        pending_bg_tasks = session.pending_bg_task_ids.len(),
                        pending_agent_tasks = session.pending_agent_task_ids.len(),
                    );
                    if !dry_run {
                        let expected_updated_at = session.updated_at;
                        let expected_pending_bg_task_ids = session.pending_bg_task_ids.clone();
                        let expected_pending_agent_task_ids =
                            session.pending_agent_task_ids.clone();
                        let mut updated_session = None;
                        store::update_session_in(sessions_dir, &session.session_id, |current| {
                            if current.status != SessionStatus::Stopped
                                || current.updated_at != expected_updated_at
                                || current.pending_bg_task_ids != expected_pending_bg_task_ids
                                || current.pending_agent_task_ids != expected_pending_agent_task_ids
                            {
                                return false;
                            }
                            current.pending_bg_task_ids.clear();
                            current.pending_agent_task_ids.clear();
                            updated_session = Some(current.clone());
                            true
                        })?;
                        let Some(updated_session) = updated_session else {
                            continue;
                        };
                        session = updated_session;
                    } else {
                        session.pending_bg_task_ids.clear();
                        session.pending_agent_task_ids.clear();
                    }
                }
                Ok(Some(_)) => {}
                Ok(None) => {
                    report.active += 1;
                    continue;
                }
                Err(error) => {
                    tracing::warn!(
                        event = "agent.sweep.pane_query_failed",
                        session = %session.session_id,
                        error = %error,
                    );
                    report.active += 1;
                    continue;
                }
            }
        }

        // Use the current time while the composer contains an unsent draft,
        // so sweep cannot terminate a session whose prompt is still being
        // composed.
        // N.B. We intentionally do NOT mutate session.updated_at here --
        // the effective timestamp is only for the timeout decision, not for
        // persisting to disk.
        let draft_observation = probe.observe_draft(&session, now);
        let effective = effective_updated_at_from_observation(&session, &draft_observation);

        match auto_pause::decide_pause_with_effective(&session, now, timeout, effective) {
            PauseDecision::Pause => {
                match probe.resolve_pids_for_pause(&session, &draft_observation) {
                    Ok(Some(pids)) => {
                        if let Some(pid) = pids
                            .iter()
                            .copied()
                            .find(|pid| probe.has_remote_control(&session, *pid))
                        {
                            tracing::info!(
                                event = "agent.sweep.remote_control_active",
                                session = %session.session_id,
                                pid,
                            );
                            report.active += 1;
                        } else if pids.is_empty() {
                            if dry_run {
                                tracing::info!(
                                    event = "agent.sweep.dry_run_pause",
                                    session = %session.session_id,
                                );
                                eprintln!(
                                    "[armyknife] agent sweep (dry-run): would confirm {} as paused",
                                    session.session_id,
                                );
                                report.paused += 1;
                                continue;
                            }
                            tracing::info!(
                                event = "agent.sweep.paused",
                                session = %session.session_id,
                            );
                            if confirm_paused(sessions_dir, session, syncer)? {
                                report.paused += 1;
                            }
                        } else if dry_run {
                            for pid in &pids {
                                tracing::info!(
                                    event = "agent.sweep.dry_run_signal",
                                    session = %session.session_id,
                                    pid,
                                );
                                eprintln!(
                                    "[armyknife] agent sweep (dry-run): would signal {} (pid={pid})",
                                    session.session_id,
                                );
                            }
                            report.signaled += 1;
                        } else {
                            let mut all_exited = true;
                            let mut signal_sent = false;
                            for pid in pids {
                                tracing::info!(
                                    event = "agent.sweep.signaled",
                                    session = %session.session_id,
                                    pid,
                                );
                                match signal_session(sessions_dir, &mut session, pid, sender)? {
                                    ShutdownOutcome::Exited => signal_sent = true,
                                    ShutdownOutcome::StillRunning => {
                                        all_exited = false;
                                        signal_sent = true;
                                    }
                                    ShutdownOutcome::SessionChanged => {
                                        all_exited = false;
                                        break;
                                    }
                                }
                            }
                            if signal_sent && all_exited {
                                tracing::info!(
                                    event = "agent.sweep.paused",
                                    session = %session.session_id,
                                );
                                if confirm_paused(sessions_dir, session, syncer)? {
                                    report.paused += 1;
                                }
                            } else if signal_sent {
                                report.signaled += 1;
                            }
                        }
                    }
                    Err(error) => {
                        tracing::warn!(
                            event = "agent.sweep.pane_query_failed",
                            session = %session.session_id,
                            error = %error,
                        );
                        report.active += 1;
                    }
                    Ok(None) => {
                        tracing::warn!(
                            event = "agent.sweep.process_unresolved",
                            session = %session.session_id,
                        );
                        report.active += 1;
                    }
                }
            }
            PauseDecision::NotYetElapsed => {
                report.waiting += 1;
            }
            PauseDecision::NotStopped => {
                report.active += 1;
            }
            PauseDecision::BgTaskPending => {
                tracing::info!(
                    event = "agent.sweep.bg_task_pending",
                    session = %session.session_id,
                    pending_bg_tasks = session.pending_bg_task_ids.len(),
                    pending_agent_tasks = session.pending_agent_task_ids.len(),
                );
                report.active += 1;
            }
        }
    }

    Ok(report)
}

/// Requests shutdown and marks the session as `sweep_signaled`. Does not
/// touch `session.status` -- a request is not proof the process has exited
/// (see module docs), so the session stays `Stopped` until a later sweep
/// observes that no `claude` pid resolves for it anymore (see
/// `confirm_paused`).
///
/// Codex first receives Ctrl+D through tmux so its normal exit path restores
/// terminal modes. If it remains alive after a short grace period, or the tmux
/// request fails, SIGTERM guarantees the pause can still make progress.
/// Claude Code continues to receive SIGTERM immediately.
///
/// `sweep_signaled` lets the `SessionEnd` hook (in `hook.rs`) recognize a
/// process that exits in response to this request instead of misreading the
/// still-`Stopped` status as the user manually ending the session.
fn signal_session<S: SignalSender + GracefulQuitRequester>(
    sessions_dir: &Path,
    session: &mut Session,
    pid: u32,
    sender: &S,
) -> Result<ShutdownOutcome> {
    let first_request = !session.sweep_signaled;
    let expected_updated_at = session.updated_at;
    let expected_sweep_signaled = session.sweep_signaled;
    let mut signaled_session = None;
    store::update_session_in(sessions_dir, &session.session_id, |current| {
        if current.status != SessionStatus::Stopped
            || current.updated_at != expected_updated_at
            || current.sweep_signaled != expected_sweep_signaled
        {
            return false;
        }
        current.sweep_signaled = true;
        signaled_session = Some(current.clone());
        true
    })?;
    let Some(signaled_session) = signaled_session else {
        return Ok(ShutdownOutcome::SessionChanged);
    };
    *session = signaled_session;

    if session.engine == Engine::Codex
        && first_request
        && let Some(tmux_info) = session.tmux_info.as_ref()
    {
        let pane_id = &tmux_info.pane_id;
        match sender.request_and_wait(tmux_info.socket_path.as_deref(), pane_id, pid) {
            Ok(true) => return Ok(ShutdownOutcome::Exited),
            Ok(false) => {
                tracing::info!(
                    event = "agent.sweep.ctrl_d_timeout",
                    session = %session.session_id,
                    pane_id,
                    pid,
                );
            }
            Err(e) => {
                tracing::warn!(
                    event = "agent.sweep.ctrl_d_failed",
                    session = %session.session_id,
                    pane_id,
                    error = %e,
                );
            }
        }
    }

    // Best-effort SIGTERM. ESRCH (process already gone) is not fatal --
    // resolve_pid raced with the process exiting; the next sweep will find
    // no resolvable pid and confirm the pause then.
    if let Err(e) = sender.send(pid, libc::SIGTERM)
        && e.raw_os_error() != Some(libc::ESRCH)
    {
        tracing::warn!(
            event = "agent.sweep.sigterm_failed",
            session = %session.session_id,
            pid = pid,
            error = %e,
        );
    }

    Ok(ShutdownOutcome::StillRunning)
}

/// Flips the session status to Paused now that sweep can no longer resolve a
/// live `claude` process for it.
///
/// Also pushes the new status into the pane / window tmux options because the
/// (presumably already SIGTERM'd) Claude Code will not fire a follow-up hook
/// to do it.
fn confirm_paused<T: TmuxStatusSyncer>(
    sessions_dir: &Path,
    session: Session,
    syncer: &T,
) -> Result<bool> {
    let expected_updated_at = session.updated_at;
    let mut paused_session = None;
    store::update_session_in(sessions_dir, &session.session_id, |current| {
        if current.status != SessionStatus::Stopped || current.updated_at != expected_updated_at {
            return false;
        }
        current.status = SessionStatus::Paused;
        current.sweep_signaled = false;
        current.updated_at = Utc::now();
        paused_session = Some(current.clone());
        true
    })?;
    let Some(session) = paused_session else {
        return Ok(false);
    };

    syncer.sync(
        session.tmux_info.as_ref().map(|info| info.pane_id.as_str()),
        Some(session.status),
        Some(&session),
        None,
        sessions_dir,
    );

    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::cell::RefCell;
    use std::collections::{BTreeSet, HashMap, HashSet};
    use std::path::PathBuf;

    use chrono::{DateTime, TimeDelta, Utc};
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::super::signal::test_support::RecordingSender;
    use super::super::store::save_session_to;
    use super::super::tmux_sync::test_support::RecordingTmuxStatusSyncer;
    use super::super::types::{Session, SessionStatus, TmuxInfo};
    use super::*;
    use crate::shared::active_session::{DraftObservation, DraftProbe};

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

    /// Test double: maps session IDs to pids and draft presence without
    /// spawning processes or touching tmux.
    #[derive(Default)]
    struct FakeProbe {
        pids: RefCell<HashMap<String, Vec<u32>>>,
        drafts: RefCell<HashSet<String>>,
        codex_process_states: RefCell<HashMap<String, bool>>,
        failed_queries: RefCell<HashSet<String>>,
        unresolved_sessions: HashSet<String>,
        remote_control_pids: HashSet<u32>,
    }

    struct ResumingProbe {
        sessions_dir: PathBuf,
    }

    struct ReactivatingProbe {
        sessions_dir: PathBuf,
    }

    impl FakeProbe {
        fn with_pids(pairs: &[(&str, u32)]) -> Self {
            let r = Self::default();
            for (id, pid) in pairs {
                r.pids.borrow_mut().insert((*id).to_string(), vec![*pid]);
            }
            r
        }

        fn with_pid_lists(pairs: &[(&str, &[u32])]) -> Self {
            let r = Self::default();
            for (id, pids) in pairs {
                r.pids.borrow_mut().insert((*id).to_string(), pids.to_vec());
            }
            r
        }

        fn with_drafts(mut self, session_ids: &[&str]) -> Self {
            for session_id in session_ids {
                self.drafts.get_mut().insert((*session_id).to_string());
            }
            self
        }

        fn with_codex_process_states(mut self, states: &[(&str, bool)]) -> Self {
            for (session_id, is_running) in states {
                self.codex_process_states
                    .get_mut()
                    .insert((*session_id).to_string(), *is_running);
            }
            self
        }

        fn with_failed_queries(mut self, session_ids: &[&str]) -> Self {
            for session_id in session_ids {
                self.failed_queries
                    .get_mut()
                    .insert((*session_id).to_string());
            }
            self
        }

        fn with_unresolved_sessions(mut self, session_ids: &[&str]) -> Self {
            self.unresolved_sessions
                .extend(session_ids.iter().map(|id| (*id).to_string()));
            self
        }

        fn with_remote_control(mut self, pids: &[u32]) -> Self {
            self.remote_control_pids.extend(pids);
            self
        }
    }

    impl DraftProbe for FakeProbe {
        fn draft_observed_at(
            &self,
            session: &Session,
            now: DateTime<Utc>,
        ) -> Option<DateTime<Utc>> {
            self.drafts
                .borrow()
                .contains(&session.session_id)
                .then_some(now)
        }

        fn observe_draft(&self, session: &Session, now: DateTime<Utc>) -> DraftObservation {
            let codex_process_running = self
                .codex_process_states
                .borrow()
                .get(&session.session_id)
                .copied();
            DraftObservation {
                observed_at: if codex_process_running == Some(false) {
                    None
                } else {
                    self.draft_observed_at(session, now)
                },
                codex_process_running,
            }
        }
    }

    impl SessionProbe for FakeProbe {
        fn resolve_pids(&self, session: &Session) -> Result<Option<Vec<u32>>> {
            if self.unresolved_sessions.contains(&session.session_id) {
                return Ok(None);
            }
            if self.failed_queries.borrow().contains(&session.session_id) {
                return Err(anyhow::anyhow!("fixture tmux query failure"));
            }
            Ok(Some(
                self.pids
                    .borrow()
                    .get(&session.session_id)
                    .cloned()
                    .unwrap_or_default(),
            ))
        }

        fn has_remote_control(&self, _session: &Session, pid: u32) -> bool {
            self.remote_control_pids.contains(&pid)
        }
    }

    impl DraftProbe for ResumingProbe {
        fn draft_observed_at(
            &self,
            _session: &Session,
            _now: DateTime<Utc>,
        ) -> Option<DateTime<Utc>> {
            None
        }
    }

    impl SessionProbe for ResumingProbe {
        fn resolve_pids(&self, session: &Session) -> Result<Option<Vec<u32>>> {
            if session.session_id == "session-first" {
                store::update_session_in(&self.sessions_dir, "session-second", |current| {
                    current.status = SessionStatus::Running;
                    current.updated_at = Utc::now();
                    true
                })?;
                return Ok(Some(vec![4242]));
            }

            Ok(Some(Vec::new()))
        }
    }

    impl DraftProbe for ReactivatingProbe {
        fn draft_observed_at(
            &self,
            _session: &Session,
            _now: DateTime<Utc>,
        ) -> Option<DateTime<Utc>> {
            None
        }
    }

    impl SessionProbe for ReactivatingProbe {
        fn resolve_pids(&self, session: &Session) -> Result<Option<Vec<u32>>> {
            store::update_session_in(&self.sessions_dir, &session.session_id, |current| {
                current.status = SessionStatus::Running;
                current.updated_at = Utc::now();
                true
            })?;
            Ok(Some(vec![4242]))
        }
    }

    fn make_session(id: &str, status: SessionStatus, updated_at: DateTime<Utc>) -> Session {
        Session {
            launch_id: None,
            archived_at: None,
            pr_context: None,
            created_pr_urls: Vec::new(),
            session_id: id.to_string(),
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            child_launch: Default::default(),
            crit_urls: Vec::new(),
            crit_not_waiting_urls: Default::default(),
            web_uis: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: PathBuf::from("/tmp"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status,
            created_at: updated_at,
            updated_at,
            last_prompted_at: None,
            last_message: None,
            current_tool: None,
            current_tool_name: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: std::collections::BTreeSet::new(),
            pending_agent_task_ids: std::collections::BTreeSet::new(),
            pending_permission_agent_ids: std::collections::BTreeSet::new(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            agent_status: None,
            agent_status_clear_pending: false,
            engine: Engine::Claude,
            model: None,
            effort: None,
        }
    }

    fn make_codex_session(id: &str, updated_at: DateTime<Utc>) -> Session {
        Session {
            engine: Engine::Codex,
            tmux_info: Some(TmuxInfo {
                session_name: "example-session".to_string(),
                window_name: "example-window".to_string(),
                window_index: 1,
                pane_id: "%42".to_string(),
                socket_path: None,
            }),
            ..make_session(id, SessionStatus::Stopped, updated_at)
        }
    }

    #[rstest]
    #[case::auto_pause_disabled(false, false, 0, 0, None)]
    #[case::auto_pause_failed(
        true,
        true,
        1,
        0,
        Some(
            "[armyknife] agent sweep: auto-pause failed: failed to parse auto-pause timeout: invalid duration"
        )
    )]
    #[case::auto_pause_succeeded(true, false, 1, 1, None)]
    #[tokio::test]
    async fn run_sweep_passes_continues_after_auto_pause_result(
        #[case] pause_enabled: bool,
        #[case] pause_fails: bool,
        #[case] expected_pause_calls: usize,
        #[case] expected_scanned: usize,
        #[case] expected_warning: Option<&str>,
    ) {
        let pause_calls = Cell::new(0);
        let codex_status_calls = Cell::new(0);
        let close_calls = Cell::new(0);
        let warning_output = RefCell::new(None::<String>);
        let result = run_sweep_passes(
            pause_enabled,
            || {
                pause_calls.set(pause_calls.get() + 1);
                if pause_fails {
                    Err(anyhow::anyhow!("invalid duration")
                        .context("failed to parse auto-pause timeout"))
                } else {
                    Ok(SweepReport {
                        scanned: 1,
                        ..SweepReport::default()
                    })
                }
            },
            || {
                codex_status_calls.set(codex_status_calls.get() + 1);
                Ok(codex_turn_status::CodexTurnUpdates {
                    failed: 3,
                    interrupted: 0,
                })
            },
            || async {
                close_calls.set(close_calls.get() + 1);
                Ok(2)
            },
            || Ok(4),
            |warning| *warning_output.borrow_mut() = Some(warning.to_string()),
        )
        .await;

        assert_eq!(
            (
                result,
                pause_calls.get(),
                codex_status_calls.get(),
                close_calls.get(),
                warning_output.into_inner(),
            ),
            (
                (
                    SweepReport {
                        scanned: expected_scanned,
                        ..SweepReport::default()
                    },
                    codex_turn_status::CodexTurnUpdates {
                        failed: 3,
                        interrupted: 0,
                    },
                    2,
                    4,
                ),
                expected_pause_calls,
                1,
                1,
                expected_warning.map(str::to_string),
            ),
        );
    }

    #[test]
    fn run_sweep_passes_continues_after_codex_status_error() {
        use std::cell::Cell;
        use std::cell::RefCell;
        use std::task::{Context, Poll, Waker};

        use crate::shared::testing::capture_json_events;

        let pause_calls = Cell::new(0);
        let close_calls = Cell::new(0);
        let warning_output = RefCell::new(Vec::<String>::new());
        let (result, warning_events) = capture_json_events(tracing::Level::WARN, || {
            let mut passes = Box::pin(run_sweep_passes(
                false,
                || {
                    pause_calls.set(pause_calls.get() + 1);
                    Ok(SweepReport::default())
                },
                || {
                    Err(anyhow::anyhow!("session store unavailable")
                        .context("failed to read Codex sessions"))
                },
                || async {
                    close_calls.set(close_calls.get() + 1);
                    Ok(2)
                },
                || Ok(4),
                |warning| warning_output.borrow_mut().push(warning.to_string()),
            ));
            let mut context = Context::from_waker(Waker::noop());
            match std::future::Future::poll(passes.as_mut(), &mut context) {
                Poll::Ready(result) => Some(result),
                Poll::Pending => None,
            }
        });

        assert_eq!(
            (
                result,
                warning_events,
                pause_calls.get(),
                close_calls.get(),
                warning_output.into_inner(),
            ),
            (
                Some((
                    SweepReport::default(),
                    codex_turn_status::CodexTurnUpdates::default(),
                    2,
                    4,
                )),
                vec![serde_json::json!({
                    "event": "agent.sweep.codex_status_failed",
                    "error": "failed to read Codex sessions: session store unavailable",
                })],
                0,
                1,
                vec!["[armyknife] agent sweep: Codex status failed: failed to read Codex sessions: session store unavailable".to_string()],
            ),
        );
    }

    #[test]
    fn run_sweep_passes_reports_merged_session_read_error_and_returns_zero_closed() {
        use std::cell::Cell;
        use std::cell::RefCell;
        use std::task::{Context, Poll, Waker};

        use crate::shared::testing::capture_json_events;

        let codex_status_calls = Cell::new(0);
        let close_pass_calls = Cell::new(0);
        let warning_output = RefCell::new(Vec::<String>::new());
        let (result, warning_events) = capture_json_events(tracing::Level::WARN, || {
            let mut passes = Box::pin(run_sweep_passes(
                false,
                || Ok(SweepReport::default()),
                || {
                    codex_status_calls.set(codex_status_calls.get() + 1);
                    Ok(codex_turn_status::CodexTurnUpdates::default())
                },
                || {
                    close_pass_calls.set(close_pass_calls.get() + 1);
                    merged_close::close_merged_sessions_with_session_reader(false, || {
                        Err(anyhow::anyhow!("database is locked")
                            .context("opening session database"))
                    })
                },
                || Ok(4),
                |warning| warning_output.borrow_mut().push(warning.to_string()),
            ));
            let mut context = Context::from_waker(Waker::noop());
            match std::future::Future::poll(passes.as_mut(), &mut context) {
                Poll::Ready(result) => Some(result),
                Poll::Pending => None,
            }
        });

        assert_eq!(
            (
                result,
                warning_events,
                codex_status_calls.get(),
                close_pass_calls.get(),
                warning_output.into_inner(),
            ),
            (
                Some((
                    SweepReport::default(),
                    codex_turn_status::CodexTurnUpdates::default(),
                    0,
                    4,
                )),
                vec![serde_json::json!({
                    "event": "agent.sweep.sessions_read_failed",
                    "error": "opening session database: database is locked",
                })],
                1,
                1,
                vec!["[armyknife] agent sweep: merged-session close failed: opening session database: database is locked".to_string()],
            ),
        );
    }

    #[tokio::test]
    async fn run_sweep_passes_runs_redelivery_last_and_recovers_its_error() {
        let calls = RefCell::new(Vec::new());
        let warnings = RefCell::new(Vec::new());
        let result = run_sweep_passes(
            true,
            || {
                calls.borrow_mut().push("pause");
                Ok(SweepReport::default())
            },
            || {
                calls.borrow_mut().push("codex_status");
                Ok(codex_turn_status::CodexTurnUpdates::default())
            },
            || async {
                calls.borrow_mut().push("merged_close");
                Ok(2)
            },
            || {
                calls.borrow_mut().push("redelivery");
                Err(anyhow::anyhow!("fixture scan failure"))
            },
            |warning| warnings.borrow_mut().push(warning.to_string()),
        )
        .await;

        assert_eq!(
            (result, calls.into_inner(), warnings.into_inner()),
            (
                (
                    SweepReport::default(),
                    codex_turn_status::CodexTurnUpdates::default(),
                    2,
                    0,
                ),
                vec!["pause", "codex_status", "merged_close", "redelivery"],
                vec![
                    "[armyknife] agent sweep: undelivered notification redelivery failed: fixture scan failure"
                        .to_string()
                ],
            ),
        );
    }

    type SweepObservation = (
        SweepReport,
        Vec<(Option<String>, String)>,
        Vec<(u32, i32)>,
        SessionStatus,
        bool,
    );

    fn observe_codex_sweep(
        test_dir: &TestDir,
        sender: &RecordingSender,
        session: Session,
        dry_run: bool,
    ) -> SweepObservation {
        let session_id = session.session_id.clone();
        save_session_to(&test_dir.path, &session).expect("save");
        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(1),
            sender,
            &FakeProbe::with_pids(&[(&session_id, 4242)]),
            &RecordingTmuxStatusSyncer::default(),
            dry_run,
        )
        .expect("sweep");
        let reloaded = store::load_session_from(&test_dir.path, &session_id)
            .expect("load")
            .expect("session exists");

        (
            report,
            sender.ctrl_d_calls.borrow().clone(),
            sender.calls.borrow().clone(),
            reloaded.status,
            reloaded.sweep_signaled,
        )
    }

    #[rstest]
    fn codex_graceful_exit_restores_terminal_before_pause(test_dir: TestDir) {
        let old = Utc::now() - TimeDelta::hours(1);
        let sender = RecordingSender::default();
        sender.graceful_exit_result.replace(true);
        let mut session = make_codex_session("codex", old);
        if let Some(tmux_info) = session.tmux_info.as_mut() {
            tmux_info.socket_path = Some("/tmp/armyknife-test/server.sock".to_string());
        }
        let actual = observe_codex_sweep(&test_dir, &sender, session, false);

        assert_eq!(
            actual,
            (
                SweepReport {
                    scanned: 1,
                    paused: 1,
                    signaled: 0,
                    waiting: 0,
                    active: 0,
                },
                vec![(
                    Some("/tmp/armyknife-test/server.sock".to_string()),
                    "%42".to_string(),
                )],
                Vec::new(),
                SessionStatus::Paused,
                false,
            )
        );
    }

    #[rstest]
    #[case::grace_period_elapsed(false, false, true)]
    #[case::tmux_request_failed(true, false, true)]
    #[case::already_signaled(false, true, false)]
    #[case::pane_not_recorded(false, false, false)]
    fn codex_falls_back_to_sigterm(
        test_dir: TestDir,
        #[case] ctrl_d_fails: bool,
        #[case] already_signaled: bool,
        #[case] has_tmux_info: bool,
    ) {
        let old = Utc::now() - TimeDelta::hours(1);
        let mut session = make_codex_session("codex", old);
        session.sweep_signaled = already_signaled;
        if !has_tmux_info {
            session.tmux_info = None;
        }

        let sender = RecordingSender::default();
        sender.fail_ctrl_d.replace(ctrl_d_fails);
        let actual = observe_codex_sweep(&test_dir, &sender, session, false);
        let expected_ctrl_d_calls = has_tmux_info
            .then(|| (None, "%42".to_string()))
            .filter(|_| !already_signaled)
            .into_iter()
            .collect::<Vec<_>>();

        assert_eq!(
            actual,
            (
                SweepReport {
                    scanned: 1,
                    paused: 0,
                    signaled: 1,
                    waiting: 0,
                    active: 0,
                },
                expected_ctrl_d_calls,
                vec![(4242, libc::SIGTERM)],
                SessionStatus::Stopped,
                true,
            )
        );
    }

    #[rstest]
    fn dry_run_does_not_request_codex_shutdown(test_dir: TestDir) {
        let old = Utc::now() - TimeDelta::hours(1);
        let sender = RecordingSender::default();
        let actual =
            observe_codex_sweep(&test_dir, &sender, make_codex_session("codex", old), true);

        assert_eq!(
            actual,
            (
                SweepReport {
                    scanned: 1,
                    paused: 0,
                    signaled: 1,
                    waiting: 0,
                    active: 0,
                },
                Vec::new(),
                Vec::new(),
                SessionStatus::Stopped,
                false,
            )
        );
    }

    #[rstest]
    fn signals_each_live_process_for_a_stopped_session(test_dir: TestDir) {
        let old = Utc::now() - TimeDelta::hours(1);
        let session = make_session("sess-a", SessionStatus::Stopped, old);
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        let probe = FakeProbe::with_pid_lists(&[("sess-a", &[4242, 4343])]);
        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(1),
            &sender,
            &probe,
            &RecordingTmuxStatusSyncer::default(),
            false,
        )
        .expect("sweep");

        let reloaded = store::load_session_from(&test_dir.path, "sess-a")
            .expect("load")
            .expect("session exists");
        assert_eq!(
            (
                report,
                sender.calls.borrow().clone(),
                reloaded.status,
                reloaded.sweep_signaled,
            ),
            (
                SweepReport {
                    scanned: 1,
                    paused: 0,
                    signaled: 1,
                    waiting: 0,
                    active: 0,
                },
                vec![(4242, libc::SIGTERM), (4343, libc::SIGTERM)],
                SessionStatus::Stopped,
                true,
            ),
        );
    }

    #[rstest]
    fn remote_control_blocks_pause(test_dir: TestDir) {
        let old = Utc::now() - TimeDelta::hours(1);
        for id in ["remote", "local"] {
            save_session_to(
                &test_dir.path,
                &make_session(id, SessionStatus::Stopped, old),
            )
            .expect("save");
        }

        let sender = RecordingSender::default();
        let probe =
            FakeProbe::with_pids(&[("remote", 1111), ("local", 2222)]).with_remote_control(&[1111]);
        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(1),
            &sender,
            &probe,
            &RecordingTmuxStatusSyncer::default(),
            false,
        )
        .expect("sweep");

        let state = |id: &str| {
            let session = store::load_session_from(&test_dir.path, id)
                .expect("load")
                .expect("session exists");
            (session.status, session.sweep_signaled)
        };
        assert_eq!(
            (
                report,
                sender.calls.borrow().clone(),
                state("remote"),
                state("local"),
            ),
            (
                SweepReport {
                    scanned: 2,
                    paused: 0,
                    signaled: 1,
                    waiting: 0,
                    active: 1,
                },
                vec![(2222, libc::SIGTERM)],
                (SessionStatus::Stopped, false),
                (SessionStatus::Stopped, true),
            ),
        );
    }

    #[rstest]
    fn reloads_each_session_before_deciding_to_pause(test_dir: TestDir) {
        let first_updated_at = Utc::now() - TimeDelta::minutes(50);
        let second_updated_at = Utc::now() - TimeDelta::hours(2);
        save_session_to(
            &test_dir.path,
            &make_session("session-first", SessionStatus::Stopped, first_updated_at),
        )
        .expect("save first session");
        save_session_to(
            &test_dir.path,
            &make_session("session-second", SessionStatus::Stopped, second_updated_at),
        )
        .expect("save second session");

        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(1),
            &RecordingSender::default(),
            &ResumingProbe {
                sessions_dir: test_dir.path.clone(),
            },
            &RecordingTmuxStatusSyncer::default(),
            false,
        )
        .expect("sweep");
        let second = store::load_session_from(&test_dir.path, "session-second")
            .expect("load second session")
            .expect("second session exists");

        assert_eq!(
            (report, second.status),
            (
                SweepReport {
                    scanned: 2,
                    paused: 0,
                    signaled: 1,
                    waiting: 0,
                    active: 1,
                },
                SessionStatus::Running,
            )
        );
    }

    #[rstest]
    fn confirms_paused_once_second_sweep_finds_no_resolvable_pid(test_dir: TestDir) {
        let old = Utc::now() - TimeDelta::hours(1);
        let session = make_session("sess-a", SessionStatus::Stopped, old);
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();

        let first_pass = sweep_impl(
            &test_dir.path,
            Duration::from_secs(1),
            &sender,
            &FakeProbe::with_pids(&[("sess-a", 4242)]),
            &RecordingTmuxStatusSyncer::default(),
            false,
        )
        .expect("first sweep");
        assert_eq!(
            first_pass,
            SweepReport {
                scanned: 1,
                paused: 0,
                signaled: 1,
                waiting: 0,
                active: 0,
            }
        );

        // The process has since exited -- resolve_pid no longer finds it.
        let second_pass = sweep_impl(
            &test_dir.path,
            Duration::from_secs(1),
            &sender,
            &FakeProbe::default(),
            &RecordingTmuxStatusSyncer::default(),
            false,
        )
        .expect("second sweep");
        assert_eq!(
            second_pass,
            SweepReport {
                scanned: 1,
                paused: 1,
                signaled: 0,
                waiting: 0,
                active: 0,
            }
        );
        assert_eq!(*sender.calls.borrow(), vec![(4242, libc::SIGTERM)]);

        let reloaded = store::load_session_from(&test_dir.path, "sess-a")
            .expect("load")
            .expect("session exists");
        assert_eq!(reloaded.status, SessionStatus::Paused);
        assert!(
            !reloaded.sweep_signaled,
            "confirming Paused should clear sweep_signaled"
        );
    }

    #[rstest]
    #[case::running(SessionStatus::Running, 1)]
    #[case::waiting(SessionStatus::WaitingInput, 1)]
    #[case::already_paused(SessionStatus::Paused, 0)]
    fn counts_sessions_by_activity(
        test_dir: TestDir,
        #[case] status: SessionStatus,
        #[case] expected_active: usize,
    ) {
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
            &RecordingTmuxStatusSyncer::default(),
            false,
        )
        .expect("sweep");

        let reloaded = store::load_session_from(&test_dir.path, "sess-b")
            .expect("load")
            .expect("session exists");
        assert_eq!(
            (
                report.scanned,
                report.paused,
                report.active,
                sender.calls.borrow().clone(),
                reloaded.status,
            ),
            (1, 0, expected_active, Vec::new(), status),
        );
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
            &RecordingTmuxStatusSyncer::default(),
            false,
        )
        .expect("sweep");

        // Ended sessions are skipped entirely so the scanned count matches
        // what `a agent list` displays.
        assert_eq!(report, SweepReport::default());
        assert!(sender.calls.borrow().is_empty());
    }

    #[rstest]
    fn replaced_sessions_are_not_sweep_targets(test_dir: TestDir) {
        let old = Utc::now() - TimeDelta::hours(1);
        let mut previous = make_session("rewound-session", SessionStatus::Ended, old);
        let mut replacement =
            make_session("replacement-session", SessionStatus::Stopped, Utc::now());
        previous.ancestor_session_ids = vec!["fixture-parent".to_string()];
        replacement.ancestor_session_ids = previous.ancestor_session_ids.clone();
        super::super::store::save_rewind_sessions(&test_dir.path, &previous, &replacement)
            .expect("replacement sessions should save");

        let sender = RecordingSender::default();
        let probe = FakeProbe::with_pids(&[("rewound-session", 4242)]);
        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(1),
            &sender,
            &probe,
            &RecordingTmuxStatusSyncer::default(),
            false,
        )
        .expect("sweep");

        assert_eq!(
            report,
            SweepReport {
                scanned: 1,
                waiting: 1,
                ..SweepReport::default()
            },
        );
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
            &RecordingTmuxStatusSyncer::default(),
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
    fn confirms_paused_when_pid_no_longer_resolves(test_dir: TestDir) {
        let old = Utc::now() - TimeDelta::hours(1);
        let mut session = make_session("sess-no-pid", SessionStatus::Stopped, old);
        session.sweep_signaled = true;
        save_session_to(&test_dir.path, &session).expect("save");

        // Resolver returns no process after an earlier sweep request. No
        // additional SIGTERM is needed; confirm Paused.
        let sender = RecordingSender::default();
        let probe = FakeProbe::default();
        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(1),
            &sender,
            &probe,
            &RecordingTmuxStatusSyncer::default(),
            false,
        )
        .expect("sweep");

        assert_eq!(
            report,
            SweepReport {
                scanned: 1,
                paused: 1,
                signaled: 0,
                waiting: 0,
                active: 0,
            }
        );
        assert!(sender.calls.borrow().is_empty());

        let reloaded = store::load_session_from(&test_dir.path, "sess-no-pid")
            .expect("load")
            .expect("session exists");
        assert_eq!(reloaded.status, SessionStatus::Paused);
    }

    #[rstest]
    fn does_not_pause_stopped_session_when_process_identity_is_unknown(test_dir: TestDir) {
        let old = Utc::now() - TimeDelta::hours(1);
        let session = make_session("unresolved", SessionStatus::Stopped, old);
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        let probe = FakeProbe::default().with_unresolved_sessions(&["unresolved"]);
        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(1),
            &sender,
            &probe,
            &RecordingTmuxStatusSyncer::default(),
            false,
        )
        .expect("sweep");
        let reloaded = store::load_session_from(&test_dir.path, "unresolved")
            .expect("load")
            .expect("session exists");

        assert_eq!(
            (report, sender.calls.borrow().clone(), reloaded.status),
            (
                SweepReport {
                    scanned: 1,
                    paused: 0,
                    signaled: 0,
                    waiting: 0,
                    active: 1,
                },
                Vec::new(),
                SessionStatus::Stopped,
            ),
        );
    }

    #[rstest]
    fn waits_for_a_paused_session_hook_before_signaling_every_process(test_dir: TestDir) {
        let old = Utc::now() - TimeDelta::hours(1);
        let session = make_session("resurrected", SessionStatus::Paused, old);
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        let probe = FakeProbe::with_pid_lists(&[("resurrected", &[4242, 4343])]);
        let first_report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(1),
            &sender,
            &probe,
            &RecordingTmuxStatusSyncer::default(),
            false,
        )
        .expect("first sweep");
        let after_first = store::load_session_from(&test_dir.path, "resurrected")
            .expect("load after first sweep")
            .expect("session exists");
        let second_report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(1),
            &sender,
            &probe,
            &RecordingTmuxStatusSyncer::default(),
            false,
        )
        .expect("second sweep");
        let reloaded = store::load_session_from(&test_dir.path, "resurrected")
            .expect("load")
            .expect("session exists");

        assert_eq!(
            (
                first_report,
                after_first.sweep_signaled,
                second_report,
                sender.calls.borrow().clone(),
                reloaded.status,
                reloaded.sweep_signaled,
            ),
            (
                SweepReport {
                    scanned: 1,
                    paused: 0,
                    signaled: 0,
                    waiting: 0,
                    active: 1,
                },
                true,
                SweepReport {
                    scanned: 1,
                    paused: 0,
                    signaled: 1,
                    waiting: 0,
                    active: 0,
                },
                vec![(4242, libc::SIGTERM), (4343, libc::SIGTERM)],
                SessionStatus::Paused,
                true,
            ),
        );
    }

    #[rstest]
    fn leaves_a_paused_session_active_when_process_identity_is_unknown(test_dir: TestDir) {
        let old = Utc::now() - TimeDelta::hours(1);
        let session = make_session("unresolved-paused", SessionStatus::Paused, old);
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        let probe = FakeProbe::default().with_unresolved_sessions(&["unresolved-paused"]);
        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(1),
            &sender,
            &probe,
            &RecordingTmuxStatusSyncer::default(),
            false,
        )
        .expect("sweep");
        let reloaded = store::load_session_from(&test_dir.path, "unresolved-paused")
            .expect("load")
            .expect("session exists");

        assert_eq!(
            (
                report,
                sender.calls.borrow().clone(),
                reloaded.status,
                reloaded.sweep_signaled,
            ),
            (
                SweepReport {
                    scanned: 1,
                    paused: 0,
                    signaled: 0,
                    waiting: 0,
                    active: 1,
                },
                Vec::new(),
                SessionStatus::Paused,
                false,
            ),
        );
    }

    #[rstest]
    fn remote_control_blocks_reaping_a_paused_session(test_dir: TestDir) {
        let old = Utc::now() - TimeDelta::hours(1);
        let session = make_session("remote-paused", SessionStatus::Paused, old);
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        let probe = FakeProbe::with_pids(&[("remote-paused", 4242)]).with_remote_control(&[4242]);
        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(1),
            &sender,
            &probe,
            &RecordingTmuxStatusSyncer::default(),
            false,
        )
        .expect("sweep");
        let reloaded = store::load_session_from(&test_dir.path, "remote-paused")
            .expect("load")
            .expect("session exists");

        assert_eq!(
            (
                report,
                sender.calls.borrow().clone(),
                reloaded.status,
                reloaded.sweep_signaled
            ),
            (
                SweepReport {
                    scanned: 1,
                    paused: 0,
                    signaled: 0,
                    waiting: 0,
                    active: 1,
                },
                Vec::new(),
                SessionStatus::Paused,
                false,
            ),
        );
    }

    #[rstest]
    fn does_not_signal_a_paused_session_that_resumed_during_resolution(test_dir: TestDir) {
        let old = Utc::now() - TimeDelta::hours(1);
        let session = make_session("resumed", SessionStatus::Paused, old);
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        let probe = ReactivatingProbe {
            sessions_dir: test_dir.path.clone(),
        };
        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(1),
            &sender,
            &probe,
            &RecordingTmuxStatusSyncer::default(),
            false,
        )
        .expect("sweep");
        let reloaded = store::load_session_from(&test_dir.path, "resumed")
            .expect("load")
            .expect("session exists");

        assert_eq!(
            (report, sender.calls.borrow().clone(), reloaded.status),
            (
                SweepReport {
                    scanned: 1,
                    paused: 0,
                    signaled: 0,
                    waiting: 0,
                    active: 0,
                },
                Vec::new(),
                SessionStatus::Running,
            ),
        );
    }

    #[rstest]
    fn keeps_stopped_session_when_tmux_server_query_fails(test_dir: TestDir) {
        let old = Utc::now() - TimeDelta::hours(1);
        let mut session = make_session("sess-server-error", SessionStatus::Stopped, old);
        session.tmux_info = Some(TmuxInfo {
            session_name: "fixture-session".to_string(),
            window_name: "fixture-window".to_string(),
            window_index: 0,
            pane_id: "%731".to_string(),
            socket_path: Some("/tmp/armyknife-test/private".to_string()),
        });
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(1),
            &sender,
            &FakeProbe::default().with_failed_queries(&["sess-server-error"]),
            &RecordingTmuxStatusSyncer::default(),
            false,
        )
        .expect("sweep should continue after a server query failure");
        let reloaded = store::load_session_from(&test_dir.path, "sess-server-error")
            .expect("load")
            .expect("session exists");

        assert_eq!(
            (
                report,
                reloaded.status,
                reloaded.sweep_signaled,
                sender.calls.borrow().clone(),
            ),
            (
                SweepReport {
                    scanned: 1,
                    paused: 0,
                    signaled: 0,
                    waiting: 0,
                    active: 1,
                },
                SessionStatus::Stopped,
                false,
                Vec::new(),
            ),
        );
    }

    #[rstest]
    fn keeps_pending_task_ids_when_tmux_server_query_fails(test_dir: TestDir) {
        let old = Utc::now() - TimeDelta::hours(1);
        let mut session = make_session("sess-pending-query-error", SessionStatus::Stopped, old);
        session
            .pending_bg_task_ids
            .insert("bg-query-pending".to_string());
        session
            .pending_agent_task_ids
            .insert("agent-query-pending".to_string());
        session.tmux_info = Some(super::super::types::TmuxInfo {
            session_name: "fixture-session".to_string(),
            window_name: "fixture-window".to_string(),
            window_index: 0,
            pane_id: "%812347".to_string(),
            socket_path: Some("/tmp/armyknife-test/pending.sock".to_string()),
        });
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(1),
            &sender,
            &FakeProbe::default().with_failed_queries(&["sess-pending-query-error"]),
            &RecordingTmuxStatusSyncer::default(),
            false,
        )
        .expect("sweep should keep pending task state after a server query failure");
        let reloaded = store::load_session_from(&test_dir.path, "sess-pending-query-error")
            .expect("load")
            .expect("session exists");

        assert_eq!(
            (
                report,
                reloaded.status,
                reloaded.pending_bg_task_ids,
                reloaded.pending_agent_task_ids,
                reloaded.sweep_signaled,
                sender.calls.borrow().clone(),
            ),
            (
                SweepReport {
                    scanned: 1,
                    paused: 0,
                    signaled: 0,
                    waiting: 0,
                    active: 1,
                },
                SessionStatus::Stopped,
                std::collections::BTreeSet::from(["bg-query-pending".to_string()]),
                std::collections::BTreeSet::from(["agent-query-pending".to_string()]),
                false,
                Vec::new(),
            ),
        );
    }

    #[rstest]
    fn esrch_does_not_confirm_paused(test_dir: TestDir) {
        // resolve_pid still finds a pid, but the SIGTERM races with the
        // process actually exiting and comes back ESRCH. Confirmation is
        // driven by resolve_pid, not by the signal outcome, so status stays
        // Stopped until a later sweep observes resolve_pid returning None.
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
            &RecordingTmuxStatusSyncer::default(),
            false,
        )
        .expect("sweep");

        assert_eq!(
            report,
            SweepReport {
                scanned: 1,
                paused: 0,
                signaled: 1,
                waiting: 0,
                active: 0,
            }
        );
        assert_eq!(*sender.calls.borrow(), vec![(4242, libc::SIGTERM)]);

        let reloaded = store::load_session_from(&test_dir.path, "sess-d")
            .expect("load")
            .expect("session exists");
        assert_eq!(reloaded.status, SessionStatus::Stopped);
    }

    #[rstest]
    fn composer_draft_blocks_pause(test_dir: TestDir) {
        // Session was marked Stopped an hour ago, so the naive timeout
        // check would pause it, but its composer still contains a draft.
        let old = Utc::now() - TimeDelta::hours(1);
        let session = make_session("typing", SessionStatus::Stopped, old);
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        let probe = FakeProbe::with_pids(&[("typing", 4242)]).with_drafts(&["typing"]);

        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(60),
            &sender,
            &probe,
            &RecordingTmuxStatusSyncer::default(),
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
    fn stale_snapshot_does_not_signal_codex_after_fresh_exit_observation(test_dir: TestDir) {
        let old = Utc::now() - TimeDelta::hours(1);
        let session = make_codex_session("codex-exited", old);
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        let probe = FakeProbe::with_pids(&[("codex-exited", 4242)])
            .with_drafts(&["codex-exited"])
            .with_codex_process_states(&[("codex-exited", false)]);
        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(1),
            &sender,
            &probe,
            &RecordingTmuxStatusSyncer::default(),
            false,
        )
        .expect("sweep");
        let reloaded = store::load_session_from(&test_dir.path, "codex-exited")
            .expect("load")
            .expect("session exists");

        assert_eq!(
            (
                report,
                sender.ctrl_d_calls.borrow().clone(),
                sender.calls.borrow().clone(),
                reloaded.status,
                reloaded.sweep_signaled,
            ),
            (
                SweepReport {
                    scanned: 1,
                    paused: 1,
                    signaled: 0,
                    waiting: 0,
                    active: 0,
                },
                Vec::new(),
                Vec::new(),
                SessionStatus::Paused,
                false,
            )
        );
    }

    #[rstest]
    // Stopped well past the timeout, but a background task launched earlier
    // in the turn -- a Bash bg shell or a Task-tool subagent, per the most
    // recent Stop hook's `background_tasks` (populated by `hook.rs`; sweep
    // does not probe task liveness itself) -- has not reported completion.
    // Pausing here would SIGTERM the live claude and kill the task with it
    // -- sweep must leave it alone and count it as active.
    #[case::bash_bg_task(|s: &mut Session| { s.pending_bg_task_ids.insert("bg-1".to_string()); })]
    #[case::agent_task(|s: &mut Session| {
        s.pending_agent_task_ids.insert("agent-task-1".to_string());
    })]
    fn pending_task_blocks_pause(test_dir: TestDir, #[case] populate: fn(&mut Session)) {
        let old = Utc::now() - TimeDelta::hours(1);
        let mut session = make_session("task-busy", SessionStatus::Stopped, old);
        populate(&mut session);
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        let probe = FakeProbe::with_pids(&[("task-busy", 4242)]);
        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(60),
            &sender,
            &probe,
            &RecordingTmuxStatusSyncer::default(),
            false,
        )
        .expect("sweep");

        assert_eq!(report.paused, 0);
        assert_eq!(report.active, 1);
        assert!(
            sender.calls.borrow().is_empty(),
            "session with a pending background task must not be SIGTERM'd"
        );

        let reloaded = store::load_session_from(&test_dir.path, "task-busy")
            .expect("load")
            .expect("session exists");
        assert_eq!(reloaded.status, SessionStatus::Stopped);
    }

    #[rstest]
    // The claude process backing this session is confirmed gone (no pid
    // resolves), so a pending id left over from before it crashed/was killed
    // is stale rather than evidence of an in-flight task -- `hook.rs` will
    // never get another `Stop` for this session_id to refresh it. Sweep must
    // clear the stale id and fall through to the normal pause decision
    // instead of reporting `BgTaskPending` forever.
    #[case::bash_bg_task(|s: &mut Session| { s.pending_bg_task_ids.insert("bg-1".to_string()); })]
    #[case::agent_task(|s: &mut Session| {
        s.pending_agent_task_ids.insert("agent-task-1".to_string());
    })]
    fn stale_pending_task_is_cleared_when_process_is_gone(
        test_dir: TestDir,
        #[case] populate: fn(&mut Session),
    ) {
        let old = Utc::now() - TimeDelta::hours(1);
        let mut session = make_session("task-gone", SessionStatus::Stopped, old);
        populate(&mut session);
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        // No resolvable pid -- the process has already exited.
        let probe = FakeProbe::default();
        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(60),
            &sender,
            &probe,
            &RecordingTmuxStatusSyncer::default(),
            false,
        )
        .expect("sweep");

        assert_eq!(
            report,
            SweepReport {
                scanned: 1,
                paused: 1,
                signaled: 0,
                waiting: 0,
                active: 0,
            }
        );

        let reloaded = store::load_session_from(&test_dir.path, "task-gone")
            .expect("load")
            .expect("session exists");
        assert_eq!(
            (
                reloaded.status,
                reloaded.pending_bg_task_ids,
                reloaded.pending_agent_task_ids,
            ),
            (SessionStatus::Paused, BTreeSet::new(), BTreeSet::new()),
        );
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
            &RecordingTmuxStatusSyncer::default(),
            true,
        )
        .expect("sweep");

        assert_eq!(report.signaled, 1);
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
    fn dry_run_does_not_confirm_paused(test_dir: TestDir) {
        let old = Utc::now() - TimeDelta::hours(1);
        let session = make_session("sess-dry-confirm", SessionStatus::Stopped, old);
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        let probe = FakeProbe::default();
        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(1),
            &sender,
            &probe,
            &RecordingTmuxStatusSyncer::default(),
            true,
        )
        .expect("sweep");

        assert_eq!(
            report,
            SweepReport {
                scanned: 1,
                paused: 1,
                signaled: 0,
                waiting: 0,
                active: 0,
            }
        );

        let reloaded = store::load_session_from(&test_dir.path, "sess-dry-confirm")
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
        // absent from the map to simulate a session whose process has
        // already exited.
        let probe = FakeProbe::with_pids(&[("pausable", 4242), ("running", 4243)]);
        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(60),
            &sender,
            &probe,
            &RecordingTmuxStatusSyncer::default(),
            false,
        )
        .expect("sweep");

        // Ended is filtered before counting; the remaining 4 break down into
        // one signaled (pid still resolves), one confirmed paused (no
        // resolvable pid), one waiting (not elapsed), one active.
        assert_eq!(
            report,
            SweepReport {
                scanned: 4,
                paused: 1,
                signaled: 1,
                waiting: 1,
                active: 1,
            }
        );
        assert_eq!(*sender.calls.borrow(), vec![(4242, libc::SIGTERM)]);
    }

    #[rstest]
    fn pause_pushes_tmux_status_for_pane(test_dir: TestDir) {
        let old = Utc::now() - TimeDelta::hours(1);
        let mut session = make_session("sess-tmux", SessionStatus::Stopped, old);
        session.tmux_info = Some(TmuxInfo {
            session_name: "main".to_string(),
            window_name: "claude".to_string(),
            window_index: 0,
            pane_id: "%42".to_string(),
            socket_path: None,
        });
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        // No resolvable pid -- this run confirms Paused and must sync tmux.
        let probe = FakeProbe::default();
        let syncer = RecordingTmuxStatusSyncer::default();
        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(1),
            &sender,
            &probe,
            &syncer,
            false,
        )
        .expect("sweep");

        assert_eq!(report.paused, 1);
        assert_eq!(
            *syncer.calls.borrow(),
            vec![(
                Some("%42".to_string()),
                Some(SessionStatus::Paused),
                Some("sess-tmux".to_string()),
                test_dir.path.clone(),
            )],
        );
    }

    #[rstest]
    fn confirm_paused_keeps_a_newer_ended_session(test_dir: TestDir) {
        let old = Utc::now() - TimeDelta::hours(1);
        let stale_session = make_session("fixture-session", SessionStatus::Stopped, old);
        save_session_to(&test_dir.path, &stale_session).expect("save stale session");
        let mut rewound_session = stale_session.clone();
        rewound_session.status = SessionStatus::Ended;
        rewound_session.updated_at = old + TimeDelta::seconds(1);
        save_session_to(&test_dir.path, &rewound_session).expect("save rewound session");
        let syncer = RecordingTmuxStatusSyncer::default();

        let result = confirm_paused(&test_dir.path, stale_session, &syncer).expect("confirm pause");
        let current_session = store::load_session_from(&test_dir.path, "fixture-session")
            .expect("load session")
            .map(|session| serde_json::to_value(session).expect("serialize session"));
        let expected_session = serde_json::to_value(rewound_session).expect("serialize session");

        assert_eq!(
            (result, current_session, syncer.calls.borrow().clone()),
            (false, Some(expected_session), Vec::new()),
        );
    }

    #[rstest]
    fn signal_session_does_not_signal_a_newer_ended_session(test_dir: TestDir) {
        let old = Utc::now() - TimeDelta::hours(1);
        let mut stale_session = make_session("fixture-session", SessionStatus::Stopped, old);
        save_session_to(&test_dir.path, &stale_session).expect("save stale session");
        let mut rewound_session = stale_session.clone();
        rewound_session.status = SessionStatus::Ended;
        rewound_session.updated_at = old + TimeDelta::seconds(1);
        save_session_to(&test_dir.path, &rewound_session).expect("save rewound session");
        let sender = RecordingSender::default();

        let result = signal_session(&test_dir.path, &mut stale_session, 4242, &sender)
            .expect("signal stale session");
        let current_session = store::load_session_from(&test_dir.path, "fixture-session")
            .expect("load session")
            .map(|session| serde_json::to_value(session).expect("serialize session"));
        let expected_session = serde_json::to_value(rewound_session).expect("serialize session");

        assert_eq!(
            (result, current_session, sender.calls.borrow().clone()),
            (
                ShutdownOutcome::SessionChanged,
                Some(expected_session),
                Vec::new()
            ),
        );
    }

    #[rstest]
    fn signal_only_does_not_push_tmux_status(test_dir: TestDir) {
        // While sweep can still resolve a live pid, it must not update the
        // pane status -- only confirming Paused does that.
        let old = Utc::now() - TimeDelta::hours(1);
        let mut session = make_session("sess-tmux-signal", SessionStatus::Stopped, old);
        session.tmux_info = Some(TmuxInfo {
            session_name: "main".to_string(),
            window_name: "claude".to_string(),
            window_index: 0,
            pane_id: "%42".to_string(),
            socket_path: None,
        });
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        let probe = FakeProbe::with_pids(&[("sess-tmux-signal", 4242)]);
        let syncer = RecordingTmuxStatusSyncer::default();
        let report = sweep_impl(
            &test_dir.path,
            Duration::from_secs(1),
            &sender,
            &probe,
            &syncer,
            false,
        )
        .expect("sweep");

        assert_eq!(
            report,
            SweepReport {
                scanned: 1,
                paused: 0,
                signaled: 1,
                waiting: 0,
                active: 0,
            }
        );
        assert!(syncer.calls.borrow().is_empty());
    }

    #[rstest]
    fn pause_without_tmux_info_still_calls_syncer_with_none(test_dir: TestDir) {
        // Sessions without tmux_info (claude run outside tmux) must still go
        // through the confirm-paused flow; syncer receives pane_id=None.
        let old = Utc::now() - TimeDelta::hours(1);
        let mut session = make_session("sess-no-tmux", SessionStatus::Stopped, old);
        session.sweep_signaled = true;
        save_session_to(&test_dir.path, &session).expect("save");

        let sender = RecordingSender::default();
        // No resolvable pid -- this run confirms Paused.
        let probe = FakeProbe::default();
        let syncer = RecordingTmuxStatusSyncer::default();
        sweep_impl(
            &test_dir.path,
            Duration::from_secs(1),
            &sender,
            &probe,
            &syncer,
            false,
        )
        .expect("sweep");

        assert_eq!(
            *syncer.calls.borrow(),
            vec![(
                None,
                Some(SessionStatus::Paused),
                Some("sess-no-tmux".to_string()),
                test_dir.path.clone(),
            )],
        );
    }

    #[rstest]
    fn missing_sessions_dir_is_not_an_error(test_dir: TestDir) {
        let sender = RecordingSender::default();
        let probe = FakeProbe::default();
        let nonexistent = test_dir.path.join("does-not-exist");
        let report = sweep_impl(
            &nonexistent,
            Duration::from_secs(1),
            &sender,
            &probe,
            &RecordingTmuxStatusSyncer::default(),
            false,
        )
        .expect("sweep should succeed even if dir is missing");
        assert_eq!(report, SweepReport::default());
    }
}
