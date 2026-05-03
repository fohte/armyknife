//! `a cc auto-compact schedule` — the detached worker spawned by the Stop
//! hook.
//!
//! Lifecycle:
//! 1. Stop hook fires `spawn_in_background(session_id)`, which forks a fresh
//!    `a cc auto-compact schedule --session <id>` process and returns
//!    immediately so the hook stays well under its budget.
//! 2. Schedule reads `@armyknife-auto-compact-timer-pid` from the session's
//!    tmux pane. If a previous schedule worker is still sleeping for this
//!    pane, it is SIGTERM'd — only the most recent Stop hook should fire a
//!    compaction, otherwise an early one would race ahead with stale state.
//! 3. Schedule writes its own pid into the same option so the next Stop hook
//!    can find and cancel it.
//! 4. Schedule sleeps for `idle_timeout` (cache-friendly default of 4m30s).
//! 5. On wake-up it re-reads the session, the pty atime, and the branch
//!    merge state, and asks `decision::decide_compact` what to do. Only
//!    `Compact` triggers side effects; every other variant is logged and the
//!    worker exits cleanly.
//! 6. SIGTERM the live `claude` process (so the next `claude -r` doesn't
//!    collide with a running interactive session) and exec
//!    `claude -r <id> -p "/compact"` to perform the compaction in print
//!    mode. `ARMYKNIFE_SKIP_HOOKS=1` is set on the child so its own Stop hook
//!    doesn't recursively schedule another compaction.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use clap::Args;

use super::decision::{CompactDecision, CompactInputs, decide_compact};
use crate::commands::cc::auto_pause;
use crate::commands::cc::signal::{LibcSignalSender, SignalSender};
use crate::commands::cc::store;
use crate::commands::cc::types::{Session, SessionStatus};
use crate::infra::git::{MergeStatus, get_merge_status_for_repo, open_repo_at};
use crate::infra::process::ProcessSnapshot;
use crate::infra::tmux;
use crate::shared::config;

/// Pane option name where the currently-sleeping schedule worker records its
/// pid so that the next Stop hook can cancel it.
const TIMER_PID_OPTION: &str = "@armyknife-auto-compact-timer-pid";

/// Bound for the descendant walk that resolves the pid of the live `claude`
/// process from a pane_pid. Same value sweep uses; a shell hosting claude has
/// at most a handful of children.
const MAX_DESCENDANT_NODES: usize = 64;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ScheduleArgs {
    /// Claude Code session_id to compact when the timer fires.
    #[arg(long)]
    pub session: String,
}

/// Spawns a detached `a cc auto-compact schedule --session <id>` so the Stop
/// hook can return immediately. Errors are swallowed (logged to stderr) —
/// failing the hook for an opportunistic optimization is the wrong trade.
pub fn spawn_in_background(session_id: &str) {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[armyknife] cc auto-compact: cannot resolve current exe: {e}");
            return;
        }
    };
    // We deliberately do NOT inherit stdin/stdout/stderr from the hook;
    // detaching them lets the hook process exit while the child keeps
    // running. Attaching them to /dev/null also avoids the child holding the
    // hook's pipes open, which would block Claude Code from advancing.
    let result = Command::new(exe)
        .args(["cc", "auto-compact", "schedule", "--session", session_id])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Err(e) = result {
        eprintln!("[armyknife] cc auto-compact: failed to spawn schedule worker: {e}");
    }
}

pub async fn run(args: &ScheduleArgs) -> Result<()> {
    let cfg = config::load_config().unwrap_or_default();
    if !cfg.cc.auto_compact.enabled {
        // Hook may still spawn us if config was edited mid-flight; bail out
        // so we don't waste a sleep.
        return Ok(());
    }

    let idle_timeout =
        auto_pause::parse_duration(&cfg.cc.auto_compact.idle_timeout).with_context(|| {
            format!(
                "invalid cc.auto_compact.idle_timeout `{}`",
                cfg.cc.auto_compact.idle_timeout
            )
        })?;

    let session = match store::load_session(&args.session)? {
        Some(s) => s,
        None => {
            eprintln!(
                "[armyknife] cc auto-compact: session {} not found, exiting",
                args.session
            );
            return Ok(());
        }
    };

    // Cancel any previously-armed worker for this pane, then advertise
    // ourselves so the next Stop hook can cancel us in turn. Both ops are
    // best-effort — losing a race here just means an extra worker exits.
    let pane_id = session.tmux_info.as_ref().map(|t| t.pane_id.clone());
    if let Some(ref pane_id) = pane_id {
        cancel_previous_timer(pane_id, &LibcSignalSender);
        let pid = std::process::id().to_string();
        let _ = tmux::set_pane_option(pane_id, TIMER_PID_OPTION, &pid);
    }

    tokio::time::sleep(idle_timeout).await;

    // Re-check the pane option: a later Stop hook may have replaced our pid
    // with the next worker's. SIGTERM from `cancel_previous_timer` is
    // asynchronous, so without this barrier our wake-up can race past the
    // signal and double-fire `/compact` together with the new worker.
    if let Some(ref pane_id) = pane_id
        && !is_current_timer(pane_id)
    {
        return Ok(());
    }

    // Reload the session: it may have moved out of Stopped (user resumed,
    // sweep paused it, …) while we slept.
    let session = match store::load_session(&args.session)? {
        Some(s) => s,
        None => return Ok(()),
    };

    let last_input = pty_last_input_for(&session);
    let branch_merged = branch_merged_for(&session).await;

    let decision = decide_compact(CompactInputs {
        session: &session,
        now: Utc::now(),
        idle_timeout,
        last_input,
        branch_merged,
    });

    match decision {
        CompactDecision::Compact => execute_compaction(&session, &LibcSignalSender)?,
        other => {
            eprintln!(
                "[armyknife] cc auto-compact: skipping session {} ({:?})",
                session.session_id, other,
            );
        }
    }
    Ok(())
}

/// Reads the prior worker's pid from the pane option and SIGTERMs it.
///
/// Tests inject a `SignalSender`; production uses `LibcSignalSender`. The
/// pane option is cleared as a side effect of the next `set_pane_option`
/// call, so we don't bother unsetting it here.
fn cancel_previous_timer<S: SignalSender>(pane_id: &str, sender: &S) {
    let Some(prev) = tmux::get_pane_option(pane_id, TIMER_PID_OPTION) else {
        return;
    };
    let Some(target) = parse_cancellable_pid(&prev, std::process::id()) else {
        return;
    };
    if !is_armyknife_process(target) {
        // Pid was recycled by the OS into an unrelated process after the
        // earlier worker died without clearing the pane option. Skipping
        // is the safe move: SIGTERMing a stranger because we recognized a
        // pid number is much worse than missing a cancellation.
        return;
    }
    if let Err(e) = sender.send(target, libc::SIGTERM)
        && e.raw_os_error() != Some(libc::ESRCH)
    {
        eprintln!("[armyknife] cc auto-compact: failed to cancel prior timer pid {target}: {e}");
    }
}

/// Returns the pid that should be SIGTERM'd when cancelling the prior timer,
/// or `None` if the recorded value is unparseable or refers to our own
/// process.
///
/// "Refers to our own process" matters because a previous run that crashed
/// mid-sleep without clearing the pane option could leave its pid behind; if
/// the OS has since recycled that pid into our own, sending SIGTERM would
/// kill us.
fn parse_cancellable_pid(recorded: &str, self_pid: u32) -> Option<u32> {
    let pid: u32 = recorded.trim().parse().ok()?;
    (pid != self_pid).then_some(pid)
}

/// Returns true if `pid` currently belongs to an armyknife binary
/// (`a`). Used as a PID-recycle guard before SIGTERMing a value we read out
/// of a pane option.
fn is_armyknife_process(pid: u32) -> bool {
    let Some(snapshot) = ProcessSnapshot::capture() else {
        // Capture failed — falling back to "trust the pid" would re-open
        // the recycle hole we're trying to close, so refuse.
        return false;
    };
    snapshot.comm_basename(pid) == Some("a")
}

/// Returns true if the pane option still records our pid as the active
/// timer. Used after `tokio::time::sleep` so a worker whose cancellation
/// SIGTERM was queued but not yet delivered exits instead of double-firing
/// `/compact` alongside the worker that replaced it.
fn is_current_timer(pane_id: &str) -> bool {
    let Some(recorded) = tmux::get_pane_option(pane_id, TIMER_PID_OPTION) else {
        // Option missing means either tmux is unavailable or the worker
        // was launched without one. Both cases predate any "we got
        // replaced" signal, so behave as if we're still current.
        return true;
    };
    recorded.trim().parse::<u32>().ok() == Some(std::process::id())
}

/// Returns the pane's pty atime as a UTC timestamp, if available. Mirrors the
/// reading sweep does in `effective_updated_at`.
fn pty_last_input_for(session: &Session) -> Option<DateTime<Utc>> {
    let pane_id = &session.tmux_info.as_ref()?.pane_id;
    let ts = tmux::get_pane_last_input(pane_id)?;
    Utc.timestamp_opt(ts, 0).single()
}

/// Returns Some(true) if the session's branch has a merged PR, Some(false)
/// for any other determinable state (open / closed / no PR), or None when we
/// could not determine it (cwd is not a git repo, GitHub call failed).
async fn branch_merged_for(session: &Session) -> Option<bool> {
    let repo = open_repo_at(&session.cwd).ok()?;
    let head = repo.head().ok()?;
    let branch = head.shorthand()?.to_string();
    // Detached HEAD: `current_branch` reports "HEAD" — there is no branch to
    // associate with a PR, so treat as "not merged" and let auto-compact run.
    if branch == "HEAD" {
        return Some(false);
    }
    let status = get_merge_status_for_repo(&repo, &branch).await;
    Some(matches!(status, MergeStatus::Merged { .. }))
}

/// SIGTERMs the live `claude` process (so we don't collide with the
/// foreground session) and runs `claude -r <id> -p "/compact"` so the
/// compaction itself reuses the still-warm prompt cache.
fn execute_compaction<S: SignalSender>(session: &Session, sender: &S) -> Result<()> {
    if let Some(pid) = resolve_claude_pid(session) {
        if let Err(e) = sender.send(pid, libc::SIGTERM)
            && e.raw_os_error() != Some(libc::ESRCH)
        {
            eprintln!(
                "[armyknife] cc auto-compact: failed to SIGTERM pid {pid} for session {}: {e}",
                session.session_id,
            );
        }
        // SIGTERM is asynchronous; give claude a beat to finish flushing
        // pending writes before we re-launch it on the same session_id.
        std::thread::sleep(Duration::from_millis(500));
    }

    spawn_compact_resume(session)?;
    mark_paused(session)?;
    Ok(())
}

fn resolve_claude_pid(session: &Session) -> Option<u32> {
    let pane_id = &session.tmux_info.as_ref()?.pane_id;
    let pane_pid = tmux::get_pane_pid(pane_id)?;
    let snapshot = ProcessSnapshot::capture()?;
    snapshot.find_self_or_descendant_by_command(pane_pid, "claude", MAX_DESCENDANT_NODES)
}

fn spawn_compact_resume(session: &Session) -> Result<()> {
    let result = Command::new("claude")
        .args(["-r", &session.session_id, "-p", "/compact"])
        // ARMYKNIFE_SKIP_HOOKS=1 makes the child's own Stop hook bail out at
        // the top of `cc hook run`, so the compaction itself doesn't
        // recursively schedule another auto-compact.
        .env("ARMYKNIFE_SKIP_HOOKS", "1")
        .current_dir(&session.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();

    match result {
        Ok(_child) => Ok(()),
        Err(e) => {
            eprintln!(
                "[armyknife] cc auto-compact: failed to spawn `claude -r -p /compact` for {}: {e}",
                session.session_id,
            );
            Err(e).context("spawn claude -r")
        }
    }
}

fn mark_paused(session: &Session) -> Result<()> {
    let dir = store::sessions_dir()?;
    mark_paused_in(&dir, session)
}

fn mark_paused_in(sessions_dir: &Path, session: &Session) -> Result<()> {
    let Some(mut stored) = store::load_session_from(sessions_dir, &session.session_id)? else {
        return Ok(());
    };
    stored.status = SessionStatus::Paused;
    stored.updated_at = Utc::now();
    store::save_session_to(sessions_dir, &stored)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::types::SessionStatus;
    use chrono::Utc;
    use rstest::rstest;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[rstest]
    #[case::valid_pid("4242", 1, Some(4242))]
    #[case::trims_whitespace("  4242\n", 1, Some(4242))]
    #[case::self_pid_returns_none("4242", 4242, None)]
    #[case::not_a_number("abc", 1, None)]
    #[case::empty_string("", 1, None)]
    fn parse_cancellable_pid_cases(
        #[case] recorded: &str,
        #[case] self_pid: u32,
        #[case] expected: Option<u32>,
    ) {
        assert_eq!(parse_cancellable_pid(recorded, self_pid), expected);
    }

    fn make_session(id: &str, status: SessionStatus) -> Session {
        let now = Utc::now();
        Session {
            session_id: id.to_string(),
            cwd: PathBuf::from("/tmp"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status,
            created_at: now,
            updated_at: now,
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            last_bg_task_pending: false,
        }
    }

    #[test]
    fn mark_paused_flips_status() {
        let dir = TempDir::new().expect("temp dir");
        let session = make_session("sess", SessionStatus::Stopped);
        store::save_session_to(dir.path(), &session).expect("save");

        mark_paused_in(dir.path(), &session).expect("mark paused");

        let reloaded = store::load_session_from(dir.path(), "sess")
            .expect("load")
            .expect("session exists");
        assert_eq!(reloaded.status, SessionStatus::Paused);
    }

    #[test]
    fn mark_paused_is_noop_for_missing_session() {
        // A schedule worker may wake up after the session file has been
        // garbage-collected (very long sleep, manual cleanup). The mark step
        // must not error in that case.
        let dir = TempDir::new().expect("temp dir");
        let session = make_session("ghost", SessionStatus::Stopped);
        mark_paused_in(dir.path(), &session).expect("noop");
        // No file should be created.
        assert!(
            store::load_session_from(dir.path(), "ghost")
                .expect("load")
                .is_none()
        );
    }
}
