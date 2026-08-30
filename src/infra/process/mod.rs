//! Process-tree utilities (parent PID lookup, descendant search).
//!
//! All external-process interaction (currently `ps`) is isolated in this
//! module so that production code elsewhere can call pure functions and tests
//! can stub at the module boundary.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsStr;
use std::io::{self, Read};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::infra::external_tool::ExternalTool;
use crate::shared::command;

/// Upper bound on how long we sleep between exit checks in [`run_with_timeout`].
const MAX_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Starting sleep between exit checks, doubled after each check up to `MAX_POLL_INTERVAL`.
const INITIAL_POLL_INTERVAL: Duration = Duration::from_millis(1);

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

    // Drain stdout/stderr on background threads while polling for exit below.
    // The pipe buffer is finite (64KiB on macOS/Linux); a child that fills it
    // blocks on its next write until someone reads, so reading only after
    // exit (as `wait_with_output` does) deadlocks against the exit poll for
    // any child that writes more than one buffer's worth of output.
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
            // The child's fds are closed by now, so these are already at (or
            // imminently at) EOF; joined only to avoid returning while they
            // still hold a reference to the killed child's pipes.
            let _ = join_reader(stdout_reader);
            let _ = join_reader(stderr_reader);
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
    command::new(program).args(args).exec()
}

/// Spawns `program args...` with stdio redirected to `/dev/null` and the
/// child detached from our stdio handles, then returns immediately without
/// waiting.
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
    cmd.spawn().map(|_| ())
}

/// Spawns a detached invocation of the current binary (`std::env::current_exe`)
/// with `args`, logging an info event before the spawn and a warn event if
/// either resolving the current executable or the spawn itself fails.
///
/// Shared by hook-triggered background workers (e.g. the auto-compact
/// schedule worker, tq session deletion) that must let the hook return
/// immediately instead of waiting on a slow or optional side effect.
/// `spawn_event`/`failed_event` are the two callers' own tracing event names,
/// so each keeps its own log identity.
pub fn spawn_self_detached(spawn_event: &str, failed_event: &str, session_id: &str, args: &[&str]) {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(
                event = failed_event,
                session = session_id,
                reason = "current_exe",
                error = %e,
            );
            return;
        }
    };
    tracing::info!(event = spawn_event, session = session_id);
    if let Err(e) = spawn_detached(exe, args.iter().copied(), None, &[]) {
        tracing::warn!(
            event = failed_event,
            session = session_id,
            reason = "spawn_detached",
            error = %e,
        );
    }
}

/// Looks up the parent PID of `pid` using `ps -o ppid= -p <pid>`.
/// Returns `None` if the process is gone or `ps` fails.
pub fn get_parent_pid(pid: u32) -> Option<u32> {
    let output = command::new("ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u32>()
        .ok()
}

/// A single snapshot of the process table, taken once and queried many times.
///
/// Stores a parent → children mapping so callers can search for descendants
/// without forking `ps` for every lookup.
pub struct ProcessSnapshot {
    children: HashMap<u32, Vec<(u32, String)>>,
    parents: HashMap<u32, u32>,
    comms: HashMap<u32, String>,
}

impl ProcessSnapshot {
    /// Captures the current process table via `ps -A`.
    /// Returns `None` if `ps` fails.
    pub fn capture() -> Option<Self> {
        let output = command::new("ps")
            .args(["-A", "-o", "pid=,ppid=,comm="])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        Some(Self::from_ps_output(&text))
    }

    fn from_ps_output(text: &str) -> Self {
        let mut children: HashMap<u32, Vec<(u32, String)>> = HashMap::new();
        let mut parents: HashMap<u32, u32> = HashMap::new();
        let mut comms: HashMap<u32, String> = HashMap::new();
        for line in text.lines() {
            let mut it = line.split_whitespace();
            let Some(pid) = it.next().and_then(|s| s.parse::<u32>().ok()) else {
                continue;
            };
            let Some(ppid) = it.next().and_then(|s| s.parse::<u32>().ok()) else {
                continue;
            };
            let comm: String = it.collect::<Vec<_>>().join(" ");
            if comm.is_empty() {
                continue;
            }
            children.entry(ppid).or_default().push((pid, comm.clone()));
            parents.insert(pid, ppid);
            comms.insert(pid, comm);
        }
        Self {
            children,
            parents,
            comms,
        }
    }

    /// Returns `pid` and every ancestor up to (but not including) the point
    /// where the parent is unknown or a cycle is detected.
    ///
    /// Used to protect the calling process and the shell/tmux chain that
    /// invoked it from being killed when cleaning up processes still rooted
    /// in a worktree being deleted (e.g. `a wm delete` run from inside it).
    pub fn ancestors(&self, pid: u32) -> HashSet<u32> {
        let mut result = HashSet::new();
        result.insert(pid);
        let mut current = pid;
        while let Some(&parent) = self.parents.get(&current) {
            if !result.insert(parent) {
                break;
            }
            current = parent;
        }
        result
    }

    /// Returns the basename of the comm for `pid`, if known.
    pub fn comm_basename(&self, pid: u32) -> Option<&str> {
        self.comms
            .get(&pid)
            .map(|c| c.rsplit('/').next().unwrap_or(c.as_str()))
    }

    /// Resolves the pid of the first process in the subtree rooted at
    /// `start_pid` (inclusive) whose comm basename matches `target`.
    ///
    /// Unlike [`find_descendant_by_command`], this also considers `start_pid`
    /// itself, which matters when the pane command is `claude` directly (no
    /// shell wrapper).
    pub fn find_self_or_descendant_by_command(
        &self,
        start_pid: u32,
        target: &str,
        max_nodes: usize,
    ) -> Option<u32> {
        if self.comm_basename(start_pid) == Some(target) {
            return Some(start_pid);
        }
        self.find_descendant_by_command(start_pid, target, max_nodes)
    }

    /// BFS from `start_pid` (exclusive) looking for the first descendant whose
    /// comm basename matches `target`. Visits at most `max_nodes` processes.
    pub fn find_descendant_by_command(
        &self,
        start_pid: u32,
        target: &str,
        max_nodes: usize,
    ) -> Option<u32> {
        let mut queue: VecDeque<u32> = VecDeque::new();
        queue.push_back(start_pid);
        let mut visited = 0usize;

        while let Some(current) = queue.pop_front() {
            visited += 1;
            if visited > max_nodes {
                return None;
            }
            if let Some(kids) = self.children.get(&current) {
                for (child_pid, comm) in kids {
                    let basename = comm.rsplit('/').next().unwrap_or(comm);
                    if basename == target {
                        return Some(*child_pid);
                    }
                    queue.push_back(*child_pid);
                }
            }
        }
        None
    }
}

/// Captures the current working directory of every process visible to the
/// caller via `lsof -a -d cwd -Fpn`, keyed by pid.
///
/// Returns `None` if `lsof` is unavailable or fails.
fn list_process_cwds() -> Option<HashMap<u32, PathBuf>> {
    let output = ExternalTool::Lsof
        .command()
        .args(["-a", "-d", "cwd", "-Fpn"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(parse_lsof_cwd_output(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

/// Parses `lsof -Fpn` output, where each line is a single field prefixed
/// with its identifier letter (`p` for pid, `n` for file name; other
/// identifiers such as `f` for file descriptor are ignored).
fn parse_lsof_cwd_output(text: &str) -> HashMap<u32, PathBuf> {
    let mut result = HashMap::new();
    let mut current_pid: Option<u32> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix('p') {
            current_pid = rest.parse().ok();
        } else if let Some(rest) = line.strip_prefix('n')
            && let Some(pid) = current_pid
        {
            result.insert(pid, PathBuf::from(rest));
        }
    }
    result
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
/// to -- the shell/tmux chain that may have invoked `a wm delete` from
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
            // SAFETY: libc::kill with SIGTERM on a pgid validated by
            // find_pgids_in_path (pgid > 1) is safe.
            unsafe { libc::kill(-pgid, libc::SIGTERM) == 0 }
        })
        .count()
}

#[cfg(test)]
mod tests {
    use indoc::indoc;
    use rstest::rstest;

    use super::*;

    // These two tests spawn a real `sh` process rather than mocking the spawn/kill
    // boundary. `run_with_timeout` wraps that exact boundary (spawn a `Command`, kill
    // it if it overruns), so there is no logic to exercise without a real process on
    // the other end; `sh` is a POSIX-guaranteed shell primitive, not an optional
    // external tool like tmux/git/ps that may be absent or blocked in a sandbox.

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
            (output.status.success(), output.stdout.len(), output.stderr),
            (true, 200_000, Vec::new()),
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

    #[rstest]
    #[case::pane_pid_is_claude_basename(
        indoc! {"
            100 1 /sbin/launchd
            200 100 claude
        "},
        200,
        Some(200),
    )]
    #[case::pane_pid_is_claude_fullpath(
        indoc! {"
            100 1 /sbin/launchd
            200 100 /usr/local/bin/claude
        "},
        200,
        Some(200),
    )]
    #[case::shell_child_is_claude(
        indoc! {"
            100 1 /sbin/launchd
            200 100 /bin/zsh
            300 200 /Users/fohte/.local/bin/claude
        "},
        200,
        Some(300),
    )]
    #[case::claude_nowhere(
        indoc! {"
            100 1 /sbin/launchd
            200 100 /bin/zsh
            300 200 vim
        "},
        200,
        None,
    )]
    #[case::grandchild_is_claude(
        indoc! {"
            100 1 /sbin/launchd
            200 100 /bin/zsh
            300 200 node
            400 300 claude
        "},
        200,
        Some(400),
    )]
    fn find_self_or_descendant_by_command_cases(
        #[case] ps_output: &str,
        #[case] start_pid: u32,
        #[case] expected: Option<u32>,
    ) {
        let snapshot = ProcessSnapshot::from_ps_output(ps_output);
        let got = snapshot.find_self_or_descendant_by_command(start_pid, "claude", 64);
        assert_eq!(got, expected);
    }

    #[rstest]
    #[case::known_basename("/usr/local/bin/claude", 200, Some("claude"))]
    #[case::bare_basename("claude", 200, Some("claude"))]
    #[case::unknown_pid("claude", 999, None)]
    fn comm_basename_cases(
        #[case] comm: &str,
        #[case] query_pid: u32,
        #[case] expected: Option<&str>,
    ) {
        let ps_output = format!("200 100 {comm}\n");
        let snapshot = ProcessSnapshot::from_ps_output(&ps_output);
        assert_eq!(snapshot.comm_basename(query_pid), expected);
    }

    #[rstest]
    #[case::walks_up_to_root(
        indoc! {"
            1 0 launchd
            100 1 tmux
            200 100 zsh
            300 200 a
        "},
        300,
        &[0, 1, 100, 200, 300],
    )]
    #[case::stops_at_unknown_parent(
        indoc! {"
            200 100 zsh
            300 200 a
        "},
        300,
        &[100, 200, 300],
    )]
    #[case::single_process(
        indoc! {"
            1 0 launchd
        "},
        1,
        &[0, 1],
    )]
    fn ancestors_cases(#[case] ps_output: &str, #[case] pid: u32, #[case] expected: &[u32]) {
        let snapshot = ProcessSnapshot::from_ps_output(ps_output);
        let mut got: Vec<u32> = snapshot.ancestors(pid).into_iter().collect();
        got.sort_unstable();
        let mut expected = expected.to_vec();
        expected.sort_unstable();
        assert_eq!(got, expected);
    }

    #[rstest]
    #[case::single_process(
        indoc! {"
            p100
            fcwd
            n/repo/.worktrees/feature
        "},
        &[(100, "/repo/.worktrees/feature")],
    )]
    #[case::multiple_processes(
        indoc! {"
            p100
            fcwd
            n/repo/.worktrees/feature
            p200
            fcwd
            n/
        "},
        &[(100, "/repo/.worktrees/feature"), (200, "/")],
    )]
    #[case::empty_output("", &[])]
    fn parse_lsof_cwd_output_cases(#[case] text: &str, #[case] expected: &[(u32, &str)]) {
        let got = parse_lsof_cwd_output(text);
        let expected: HashMap<u32, PathBuf> = expected
            .iter()
            .map(|&(pid, path)| (pid, PathBuf::from(path)))
            .collect();
        assert_eq!(got, expected);
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
