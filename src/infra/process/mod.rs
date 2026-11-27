//! Process-tree and process-lifetime utilities.
//!
//! Process inspection and command execution are isolated here so production
//! code elsewhere can keep OS-level process operations at one boundary.

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::io::{self, Read};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::shared::command;

mod exit_wait;
mod identity;
mod output_tail;
mod snapshot;

use snapshot::list_process_cwds;

pub(crate) use exit_wait::{Wake, dir_entry_names, wait_for_exit_or_dir_change};
pub(crate) use identity::{process_start_time, process_start_time_matches};

pub(crate) use output_tail::{
    DrainDeadlineCheck, OutputTail, OutputTailOptions, read_output_tail, set_nonblocking,
};
pub use snapshot::{
    ProcessInPath, ProcessSnapshot, SHELL_COMMANDS, find_caller_pids_in_path,
    find_non_shell_processes_in_path, get_parent_pid,
};
pub(crate) use snapshot::{
    WorktreeProcessInspection, current_caller_pane_pid, inspect_worktree_processes,
};

pub(crate) fn new_command(program: impl AsRef<OsStr>) -> Command {
    command::new(program)
}

pub(crate) fn spawn(mut command: Command) -> io::Result<Child> {
    command.spawn()
}

/// Upper bound on how long we sleep between exit checks in [`run_with_timeout`].
const MAX_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Starting sleep between exit checks, doubled after each check up to `MAX_POLL_INTERVAL`.
const INITIAL_POLL_INTERVAL: Duration = Duration::from_millis(1);

/// Returns whether a process ID still refers to a live process.
pub fn is_process_alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if pid == 0 {
        return false;
    }

    // SAFETY: signal 0 only checks whether this process ID exists.
    let result = unsafe { libc::kill(pid, 0) };
    result == 0 || io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

/// Waits up to `timeout` for a process to exit, polling every 50 ms.
pub fn wait_for_process_exit(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while is_process_alive(pid) {
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(50));
    }
    true
}

/// Runs `command` to completion, killing it if it does not exit within `timeout`.
///
/// Callers that shell out to a service that can itself hang (a tmux server
/// stuck processing its own job queue, a CLI blocked on a gated backend) use
/// this instead of `Command::output` so a hung child fails fast with a
/// reported error instead of leaking a thread and process indefinitely.
pub fn run_with_timeout(mut command: Command, timeout: Duration) -> io::Result<Output> {
    // stdin is closed (not inherited) to match `Command::output`'s behavior, which
    // this replaces.
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Drain pipes concurrently so the child does not block on a full OS pipe
    // buffer before exiting.
    let (Some(mut stdout), Some(mut stderr)) = (child.stdout.take(), child.stderr.take()) else {
        return Err(io::Error::other(
            "child spawned without piped stdout/stderr",
        ));
    };
    let stdout_reader = thread::spawn(move || {
        let mut buf = Vec::new();
        stdout.read_to_end(&mut buf).map(|_| buf)
    });
    let stderr_reader = thread::spawn(move || {
        let mut buf = Vec::new();
        stderr.read_to_end(&mut buf).map(|_| buf)
    });

    let deadline = Instant::now() + timeout;
    let mut poll_interval = INITIAL_POLL_INTERVAL;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            // `Child::kill` is a no-op if the child was already reaped by a prior
            // `try_wait`/`wait` call, so this never targets a pid the OS has since
            // recycled for an unrelated process.
            let _ = child.kill();
            let _ = child.wait();
            // Not joined: a descendant the child spawned (e.g. a backgrounded
            // shell job) can keep a pipe's write end open past the child's
            // own exit, and joining would then block past `timeout` waiting
            // for EOF that may never come. Dropping a `JoinHandle` detaches
            // the thread instead of blocking on it.
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("command timed out after {timeout:?}"),
            ));
        }
        thread::sleep(poll_interval);
        poll_interval = std::cmp::min(poll_interval * 2, MAX_POLL_INTERVAL);
    };

    Ok(Output {
        status,
        stdout: join_reader(stdout_reader)?,
        stderr: join_reader(stderr_reader)?,
    })
}

/// Joins a pipe-reading thread spawned by [`run_with_timeout`], collapsing a
/// thread panic into an `io::Error` since callers only propagate `io::Result`.
fn join_reader(handle: thread::JoinHandle<io::Result<Vec<u8>>>) -> io::Result<Vec<u8>> {
    handle
        .join()
        .unwrap_or_else(|_| Err(io::Error::other("pipe reader thread panicked")))
}

/// Replaces the current process image with `program args...` via `execve(2)`.
/// Returns only on failure; the returned `io::Error` describes why `exec` could not start the program.
pub fn exec_replace<P, I, S>(program: P, args: I) -> io::Error
where
    P: AsRef<std::ffi::OsStr>,
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    exec_replace_with_env(program, args, &[])
}

/// Like [`exec_replace`], but adds `extra_env` on top of the inherited
/// environment before the `execve(2)` call.
pub fn exec_replace_with_env<P, I, S>(program: P, args: I, extra_env: &[(&str, &str)]) -> io::Error
where
    P: AsRef<std::ffi::OsStr>,
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut cmd = command::new(program);
    cmd.args(args);
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    cmd.exec()
}

/// Spawns `program args...` in a new session with stdio redirected to
/// `/dev/null`, then returns immediately without waiting.
///
/// Used for fire-and-forget background workers (e.g., the auto-compact
/// schedule worker spawned from the Stop hook): the parent process must be
/// able to exit while the child keeps running, and the parent's pipes must
/// not be held open by the child or upstream callers (Claude Code's hook
/// runner) would block on EOF.
///
/// `cwd` and `extra_env` apply to the child only.
pub fn spawn_detached<P, I, S>(
    program: P,
    args: I,
    cwd: Option<&Path>,
    extra_env: &[(&str, &str)],
) -> io::Result<()>
where
    P: AsRef<OsStr>,
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    spawn_detached_with_env_overrides(program, args, cwd, extra_env, &[])
}

/// Spawns a detached process with explicit additions and removals from the
/// inherited environment.
pub fn spawn_detached_with_env_overrides<P, I, S>(
    program: P,
    args: I,
    cwd: Option<&Path>,
    extra_env: &[(&str, &str)],
    removed_env: &[&str],
) -> io::Result<()>
where
    P: AsRef<OsStr>,
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    detached_command(program, args, cwd, extra_env, removed_env)
        .spawn()
        .map(|_| ())
}

/// Like [`spawn_detached`], but returns the detached child PID so callers can
/// track its lifetime without keeping a pipe or waiting on the child.
pub fn spawn_detached_with_pid<P, I, S>(
    program: P,
    args: I,
    cwd: Option<&Path>,
    extra_env: &[(&str, &str)],
) -> io::Result<u32>
where
    P: AsRef<OsStr>,
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    detached_command(program, args, cwd, extra_env, &[])
        .spawn()
        .map(|child| child.id())
}

fn detached_command<P, I, S>(
    program: P,
    args: I,
    cwd: Option<&Path>,
    extra_env: &[(&str, &str)],
    removed_env: &[&str],
) -> Command
where
    P: AsRef<OsStr>,
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    for key in removed_env {
        cmd.env_remove(key);
    }

    // SAFETY: `setsid` only manipulates the calling process's session
    // membership; it is async-signal-safe and documented as one of the
    // operations safe to call in `pre_exec`. A separate session keeps
    // fire-and-forget workers out of the hook runner's process group.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    cmd
}

/// Spawns a detached invocation of the current binary (`std::env::current_exe`)
/// with `args`, logging an info event before the spawn and a warn event if
/// either resolving the current executable or the spawn itself fails.
///
/// Shared by hook-triggered background workers (e.g. the auto-compact
/// schedule worker, tq session archive worker) that must let the hook return
/// immediately instead of waiting on a slow or optional side effect.
/// `spawn_event`/`failed_event` are the two callers' own tracing event names,
/// so each keeps its own log identity.
pub fn spawn_self_detached(spawn_event: &str, failed_event: &str, session_id: &str, args: &[&str]) {
    spawn_self_detached_with_subject(
        spawn_event,
        failed_event,
        DetachedSubject::Session(session_id),
        args,
    );
}

/// Spawns a detached invocation associated with a repository rather than a
/// session, keeping the repository path under its own tracing field.
pub fn spawn_self_detached_for_repo(
    spawn_event: &str,
    failed_event: &str,
    repo_path: &Path,
    args: &[&str],
) {
    spawn_self_detached_with_subject(
        spawn_event,
        failed_event,
        DetachedSubject::Repository(repo_path),
        args,
    );
}

enum DetachedSubject<'a> {
    Session(&'a str),
    Repository(&'a Path),
}

fn spawn_self_detached_with_subject(
    spawn_event: &str,
    failed_event: &str,
    subject: DetachedSubject<'_>,
    args: &[&str],
) {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            log_detached_spawn_failure(failed_event, &subject, "current_exe", &e);
            return;
        }
    };
    log_detached_spawn_started(spawn_event, &subject);
    if let Err(e) = spawn_detached(exe, args.iter().copied(), None, &[]) {
        log_detached_spawn_failure(failed_event, &subject, "spawn_detached", &e);
    }
}

fn log_detached_spawn_started(event: &str, subject: &DetachedSubject<'_>) {
    match subject {
        DetachedSubject::Session(session_id) => {
            tracing::info!(event, session = session_id);
        }
        DetachedSubject::Repository(repo_path) => {
            tracing::info!(event, repo = %repo_path.display());
        }
    }
}

fn log_detached_spawn_failure(
    event: &str,
    subject: &DetachedSubject<'_>,
    reason: &str,
    error: &dyn std::fmt::Display,
) {
    match subject {
        DetachedSubject::Session(session_id) => {
            tracing::warn!(event, session = session_id, reason, error = %error);
        }
        DetachedSubject::Repository(repo_path) => {
            tracing::warn!(event, repo = %repo_path.display(), reason, error = %error);
        }
    }
}

/// Returns the pids in `cwds` whose value is inside `path`, excluding any
/// pid in `exclude`.
fn filter_pids_in_path(
    cwds: &HashMap<u32, PathBuf>,
    path: &Path,
    exclude: &HashSet<u32>,
) -> Vec<u32> {
    cwds.iter()
        .filter(|(pid, cwd)| !exclude.contains(pid) && cwd.starts_with(path))
        .map(|(&pid, _)| pid)
        .collect()
}

/// Returns the pgid of `pid` via `getpgid(2)`. `None` if the process is gone
/// or otherwise inaccessible.
fn get_pgid(pid: u32) -> Option<libc::pid_t> {
    // SAFETY: getpgid with any pid is safe to call; it returns -1 on error
    // (e.g. ESRCH for a gone process), which is treated as "unknown".
    let pgid = unsafe { libc::getpgid(pid as libc::pid_t) };
    if pgid == -1 { None } else { Some(pgid) }
}

/// Finds the process-group ids (pgid) of processes rooted in `path`,
/// excluding any pgid the calling process or one of its ancestors belongs
/// to -- the shell/tmux chain that may have invoked `a agent close` from
/// inside the worktree being deleted.
///
/// A pid's cwd is used only to *locate* a candidate group: once one member
/// is found inside `path`, the whole pgid is returned so the caller can kill
/// the entire tree, including members whose own cwd has since moved
/// elsewhere (e.g. a package-manager wrapper that cd'd into a subdirectory
/// before exec'ing its target). This relies on Claude Code's
/// background-spawn boundary putting the whole detached subtree into one
/// process group; it doesn't hold if a member calls `setpgid` itself.
///
/// Returns an empty vec if `lsof` or `ps` is unavailable, since without a
/// full process snapshot the ancestor chain can't be computed safely.
pub fn find_pgids_in_path(path: &Path) -> Vec<libc::pid_t> {
    let Some(cwds) = list_process_cwds() else {
        return Vec::new();
    };
    let Some(snapshot) = ProcessSnapshot::capture() else {
        return Vec::new();
    };
    let exclude_pids = snapshot.ancestors(std::process::id());
    // lsof reports each process's cwd fully resolved (e.g. macOS's
    // /tmp -> /private/tmp), so `path` must be resolved the same way or a
    // symlinked worktree root would never match.
    let resolved_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let candidate_pids = filter_pids_in_path(&cwds, &resolved_path, &exclude_pids);

    let exclude_pgids: HashSet<libc::pid_t> = exclude_pids
        .iter()
        .filter_map(|&pid| get_pgid(pid))
        .collect();

    candidate_pids
        .iter()
        .filter_map(|&pid| get_pgid(pid))
        // Guard against special pgids (0 = caller's group, -1 = broadcast) in kill(2).
        .filter(|&pgid| pgid > 1 && !exclude_pgids.contains(&pgid))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect()
}

/// Sends SIGTERM to every process in each group via `kill(-pgid, ...)`.
/// Returns the number of process groups signaled successfully.
pub fn kill_process_groups(pgids: &[libc::pid_t]) -> usize {
    pgids
        .iter()
        .filter(|&&pgid| {
            // SAFETY: callers provide process groups validated as greater than 1.
            unsafe { libc::kill(-pgid, libc::SIGTERM) == 0 }
        })
        .count()
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use std::ffi::OsStr;

    use super::*;

    #[test]
    fn detached_title_generation_removes_claudecode() {
        let command = detached_command(
            "unused-program",
            std::iter::empty::<&str>(),
            None,
            &[],
            &["CLAUDECODE"],
        );

        assert_eq!(
            command.get_envs().collect::<Vec<_>>(),
            vec![(OsStr::new("CLAUDECODE"), None)],
        );
    }

    // `setsid` is only observable after the spawn boundary, so this test uses
    // the POSIX-guaranteed `sh` rather than an optional external tool.
    #[test]
    fn spawn_detached_starts_child_in_a_new_session() {
        let pid_file = tempfile::NamedTempFile::new().expect("pid file should be created");
        let pid_path = pid_file.path().as_os_str();

        spawn_detached(
            "sh",
            [
                "-c".as_ref(),
                "printf %s \"$$\" > \"$1\"; kill -STOP $$".as_ref(),
                "sh".as_ref(),
                pid_path,
            ],
            None,
            &[],
        )
        .expect("child should spawn");

        let deadline = Instant::now() + Duration::from_secs(2);
        let child_pid = loop {
            let contents =
                std::fs::read_to_string(pid_file.path()).expect("pid file should be readable");
            if let Ok(pid) = contents.parse::<libc::pid_t>() {
                break Ok(pid);
            }
            if Instant::now() >= deadline {
                break Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "child did not write its pid promptly",
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
        .expect("child should write its pid promptly");

        // SAFETY: both calls only query process membership.
        let (child_pgid, child_sid) =
            unsafe { (libc::getpgid(child_pid), libc::getsid(child_pid)) };
        // SAFETY: the child is stopped and this pid came directly from that child.
        unsafe {
            libc::kill(child_pid, libc::SIGKILL);
        }

        assert_eq!((child_pgid, child_sid), (child_pid, child_pid));
    }

    // These `run_with_timeout` tests spawn a real `sh` process rather than mocking
    // the spawn/kill boundary. `run_with_timeout` wraps that exact boundary (spawn a
    // `Command`, kill it if it overruns), so there is no logic to exercise without a
    // real process on the other end; `sh` is a POSIX-guaranteed shell primitive, not
    // an optional external tool like tmux/git/ps that may be absent or blocked in a
    // sandbox.

    #[test]
    fn run_with_timeout_returns_output_when_command_finishes_in_time() {
        let mut command = Command::new("sh");
        command.args(["-c", "printf ok"]);

        let output =
            run_with_timeout(command, Duration::from_secs(5)).expect("command should not time out");

        assert_eq!(
            (output.status.success(), output.stdout, output.stderr),
            (true, b"ok".to_vec(), Vec::new()),
        );
    }

    #[test]
    fn run_with_timeout_reads_output_larger_than_pipe_buffer_without_deadlocking() {
        // 200_000 bytes comfortably exceeds the 64KiB pipe buffer that a child
        // fills before the parent has read anything; a parent that reads only
        // after the child exits would deadlock here instead of returning.
        let mut command = Command::new("sh");
        command.args(["-c", "head -c 200000 /dev/zero"]);

        let start = Instant::now();
        let output = run_with_timeout(command, Duration::from_secs(10))
            .expect("command should not time out");
        let elapsed = start.elapsed();

        assert_eq!(
            (output.status.success(), output.stdout, output.stderr),
            (true, vec![0u8; 200_000], Vec::new()),
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "expected large output to return promptly instead of blocking until timeout, took {elapsed:?}"
        );
    }

    #[test]
    fn run_with_timeout_kills_and_errors_when_command_exceeds_timeout() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 5"]);

        let start = Instant::now();
        let result = run_with_timeout(command, Duration::from_millis(100));
        let elapsed = start.elapsed();

        let err = result.expect_err("command should time out");
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(
            elapsed < Duration::from_secs(2),
            "expected the timeout to cut the 5s sleep short, took {elapsed:?}"
        );
    }

    #[test]
    fn run_with_timeout_returns_promptly_when_descendant_holds_pipe_open() {
        // The backgrounded `sleep 3` is reparented but keeps the stdout/stderr
        // fds it inherited from `sh`, so the pipes stay open for ~3s after `sh`
        // itself is killed. Returning promptly here (instead of blocking on
        // that descendant's EOF) is what makes this function fail fast.
        let mut command = Command::new("sh");
        command.args(["-c", "(sleep 3 &) ; sleep 3"]);

        let start = Instant::now();
        let result = run_with_timeout(command, Duration::from_millis(100));
        let elapsed = start.elapsed();

        let err = result.expect_err("command should time out");
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(
            elapsed < Duration::from_secs(2),
            "expected the timeout to return promptly instead of blocking on the descendant's pipe, took {elapsed:?}"
        );
    }

    #[rstest]
    fn filter_pids_in_path_excludes_outside_path_and_excluded_pids() {
        let cwds: HashMap<u32, PathBuf> = [
            (100, PathBuf::from("/repo/.worktrees/feature")),
            (200, PathBuf::from("/repo/.worktrees/feature/web")),
            (300, PathBuf::from("/repo/.worktrees/other")),
            (400, PathBuf::from("/repo/.worktrees/feature")),
        ]
        .into_iter()
        .collect();
        let exclude: HashSet<u32> = [400].into_iter().collect();

        let mut got = filter_pids_in_path(&cwds, Path::new("/repo/.worktrees/feature"), &exclude);
        got.sort_unstable();

        assert_eq!(got, vec![100, 200]);
    }
}
