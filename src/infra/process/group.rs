use std::collections::VecDeque;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::{INITIAL_POLL_INTERVAL, MAX_POLL_INTERVAL};

const PROCESS_GROUP_TERMINATION_GRACE: Duration = Duration::from_millis(250);
const STDERR_READER_POLL_INTERVAL: Duration = Duration::from_millis(10);
const STDERR_DRAIN_TIMEOUT: Duration = Duration::from_millis(100);

/// Process outcome plus the bounded stderr tail, including when the process times out.
pub struct ProcessGroupOutput {
    pub status: io::Result<ExitStatus>,
    pub stderr: Vec<u8>,
}

static GROUP_RUNNER_CANCEL_SIGNAL: AtomicI32 = AtomicI32::new(0);
static GROUP_RUNNER_LOCK: Mutex<()> = Mutex::new(());

struct SignalHandlerGuard {
    previous_sigint: libc::sighandler_t,
    previous_sigterm: libc::sighandler_t,
}

struct ProcessGroupGuard {
    pgid: libc::pid_t,
    active: bool,
}

impl ProcessGroupGuard {
    fn terminate(&mut self) {
        if self.active {
            terminate_process_group(self.pgid);
            self.active = false;
        }
    }
}

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        self.terminate();
    }
}

impl Drop for SignalHandlerGuard {
    fn drop(&mut self) {
        // SAFETY: the guard restores the process-wide handlers it replaced.
        unsafe {
            libc::signal(libc::SIGINT, self.previous_sigint);
            libc::signal(libc::SIGTERM, self.previous_sigterm);
        }
    }
}

extern "C" fn record_group_runner_signal(signal: libc::c_int) {
    GROUP_RUNNER_CANCEL_SIGNAL.store(signal, Ordering::Relaxed);
}

/// Runs a command in a new process group and terminates the group on exit,
/// timeout, or an interrupt signal.
///
/// The child's standard streams are redirected to `/dev/null`; callers that
/// need output should use a separate bounded output channel.
pub fn run_in_process_group_with_timeout(
    command: Command,
    timeout: Duration,
) -> io::Result<ExitStatus> {
    run_in_process_group(command, timeout, None).and_then(|output| output.status)
}

/// Runs a command in a new process group and returns only the bounded tail of
/// its stderr alongside the exit status.
pub fn run_in_process_group_with_timeout_and_stderr(
    command: Command,
    timeout: Duration,
    max_stderr_bytes: usize,
) -> io::Result<ProcessGroupOutput> {
    run_in_process_group(command, timeout, Some(max_stderr_bytes))
}

fn run_in_process_group(
    mut command: Command,
    timeout: Duration,
    max_stderr_bytes: Option<usize>,
) -> io::Result<ProcessGroupOutput> {
    let _runner_lock = GROUP_RUNNER_LOCK
        .lock()
        .map_err(|_| io::Error::other("process-group runner lock was poisoned"))?;
    GROUP_RUNNER_CANCEL_SIGNAL.store(0, Ordering::Relaxed);
    let _signal_guard = install_group_runner_signal_handlers()?;
    command.stdin(Stdio::null()).stdout(Stdio::null());
    command.stderr(if max_stderr_bytes.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    command.process_group(0);
    let mut child = command.spawn()?;
    let pid = match libc::pid_t::try_from(child.id()) {
        Ok(pid) => pid,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::other(
                "child PID is outside the process-group range",
            ));
        }
    };
    let mut process_group = ProcessGroupGuard {
        pgid: pid,
        active: true,
    };
    let (stderr_reader, stderr_cancel) = if let Some(max_stderr_bytes) = max_stderr_bytes {
        let Some(stderr) = child.stderr.take() else {
            process_group.terminate();
            let _ = child.wait();
            return Err(io::Error::other("child stderr pipe was unavailable"));
        };
        if let Err(error) = set_nonblocking(&stderr) {
            process_group.terminate();
            let _ = child.wait();
            return Err(error);
        }
        let cancel_reader = Arc::new(AtomicBool::new(false));
        let thread_cancel = cancel_reader.clone();
        match thread::Builder::new()
            .name("armyknife-process-stderr-reader".to_string())
            .spawn(move || read_stderr_tail(stderr, max_stderr_bytes, thread_cancel))
        {
            Ok(reader) => (Some(reader), Some(cancel_reader)),
            Err(error) => {
                process_group.terminate();
                let _ = child.wait();
                return Err(error);
            }
        }
    } else {
        (None, None)
    };
    let deadline = Instant::now() + timeout;
    let mut poll_interval = INITIAL_POLL_INTERVAL;

    let status = loop {
        if GROUP_RUNNER_CANCEL_SIGNAL.load(Ordering::Relaxed) != 0 {
            process_group.terminate();
            let _ = child.wait();
            break Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "command interrupted",
            ));
        }

        match child_has_exited_without_reaping(pid) {
            Ok(true) => {
                process_group.terminate();
                break child.wait();
            }
            Ok(false) => {}
            Err(error) => {
                process_group.terminate();
                let _ = child.wait();
                break Err(error);
            }
        }

        if Instant::now() >= deadline {
            process_group.terminate();
            let _ = child.wait();
            break Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("command timed out after {timeout:?}"),
            ));
        }

        thread::sleep(poll_interval);
        poll_interval = std::cmp::min(poll_interval * 2, MAX_POLL_INTERVAL);
    };

    if let Some(cancel_reader) = stderr_cancel {
        cancel_reader.store(true, Ordering::Relaxed);
    }
    let stderr = stderr_reader
        .map(|reader| {
            reader
                .join()
                .map_err(|_| io::Error::other("stderr reader thread panicked"))?
        })
        .transpose()?;
    Ok(ProcessGroupOutput {
        status,
        stderr: stderr.unwrap_or_default(),
    })
}

fn set_nonblocking(reader: &impl AsRawFd) -> io::Result<()> {
    let descriptor = reader.as_raw_fd();
    // SAFETY: `descriptor` is the valid stderr pipe owned by the child handle.
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: F_SETFL updates only the status flags of the valid stderr pipe.
    let result = unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if result == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn read_stderr_tail(
    mut reader: impl io::Read,
    max_bytes: usize,
    cancelled: Arc<AtomicBool>,
) -> io::Result<Vec<u8>> {
    let mut tail = VecDeque::with_capacity(max_bytes);
    let mut chunk = [0; 4096];
    let mut drain_deadline = None;
    loop {
        if cancelled.load(Ordering::Relaxed)
            && Instant::now()
                >= *drain_deadline.get_or_insert_with(|| Instant::now() + STDERR_DRAIN_TIMEOUT)
        {
            break;
        }
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                if max_bytes == 0 {
                    continue;
                }
                for byte in &chunk[..read] {
                    if tail.len() == max_bytes {
                        tail.pop_front();
                    }
                    tail.push_back(*byte);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if cancelled.load(Ordering::Relaxed) {
                    break;
                }
                thread::sleep(STDERR_READER_POLL_INTERVAL);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(tail.into_iter().collect())
}

fn install_group_runner_signal_handlers() -> io::Result<SignalHandlerGuard> {
    // SAFETY: the handler only stores an integer in an atomic and is installed
    // for the duration of one foreground inference child.
    let previous_sigint = unsafe {
        libc::signal(
            libc::SIGINT,
            record_group_runner_signal as *const () as libc::sighandler_t,
        )
    };
    if previous_sigint == libc::SIG_ERR {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: the handler only stores an integer in an atomic and is installed
    // for the duration of one foreground inference child.
    let previous_sigterm = unsafe {
        libc::signal(
            libc::SIGTERM,
            record_group_runner_signal as *const () as libc::sighandler_t,
        )
    };
    if previous_sigterm == libc::SIG_ERR {
        // SAFETY: restore the first handler because the second registration failed.
        unsafe { libc::signal(libc::SIGINT, previous_sigint) };
        return Err(io::Error::last_os_error());
    }

    Ok(SignalHandlerGuard {
        previous_sigint,
        previous_sigterm,
    })
}

fn child_has_exited_without_reaping(pid: libc::pid_t) -> io::Result<bool> {
    // SAFETY: waitid only observes the child identified by the PID returned from
    // Command::spawn. WNOWAIT keeps its process-group ID from being reused until
    // group cleanup has finished.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    // SAFETY: `info` is writable and the flags request a non-blocking observation.
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result == -1 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            return Ok(false);
        }
        return Err(error);
    }
    // SAFETY: waitid initialized siginfo_t on success; si_pid is valid to read.
    Ok(unsafe { info.si_pid() } != 0)
}

fn terminate_process_group(pgid: libc::pid_t) {
    if pgid <= 1 {
        return;
    }
    // SAFETY: CommandExt::process_group(0) placed only this command and its
    // descendants in the group identified by its PID.
    unsafe { libc::kill(-pgid, libc::SIGTERM) };
    thread::sleep(PROCESS_GROUP_TERMINATION_GRACE);
    // SAFETY: the process group remains reserved while the direct child is a
    // waitable zombie, so this cannot target a recycled process group ID.
    unsafe { libc::kill(-pgid, libc::SIGKILL) };
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::infra::process::is_process_alive;

    const GROUP_HELPER_ROLE_ENV: &str = "ARMYKNIFE_PROCESS_GROUP_HELPER_ROLE";
    const GROUP_HELPER_MODE_ENV: &str = "ARMYKNIFE_PROCESS_GROUP_HELPER_MODE";
    const GROUP_HELPER_PID_FILE_ENV: &str = "ARMYKNIFE_PROCESS_GROUP_HELPER_PID_FILE";
    const GROUP_HELPER_READY_FILE_ENV: &str = "ARMYKNIFE_PROCESS_GROUP_HELPER_READY_FILE";
    const GROUP_HELPER_STDERR_ENV: &str = "ARMYKNIFE_PROCESS_GROUP_HELPER_STDERR";

    #[test]
    #[ignore = "subprocess fixture for process-group cleanup tests"]
    #[expect(
        clippy::zombie_processes,
        reason = "the fixture deliberately leaves a child for process-group cleanup tests"
    )]
    fn process_group_child_helper() {
        let Ok(role) = std::env::var(GROUP_HELPER_ROLE_ENV) else {
            return;
        };
        if role == "stderr" {
            eprint!(
                "{}",
                std::env::var(GROUP_HELPER_STDERR_ENV)
                    .expect("stderr helper needs its fixture output")
            );
            if let Some(ready_file) = std::env::var_os(GROUP_HELPER_READY_FILE_ENV) {
                std::fs::write(ready_file, b"ready")
                    .expect("stderr helper should record its readiness");
            }
            if std::env::var(GROUP_HELPER_MODE_ENV).as_deref() == Ok("wait") {
                loop {
                    thread::sleep(Duration::from_secs(1));
                }
            }
            return;
        }
        let pid_file = PathBuf::from(
            std::env::var_os(GROUP_HELPER_PID_FILE_ENV)
                .expect("process group helper needs a PID file"),
        );
        if role == "descendant" {
            std::fs::write(&pid_file, std::process::id().to_string())
                .expect("descendant should write its PID");
            loop {
                thread::sleep(Duration::from_secs(1));
            }
        }

        let descendant_pid_file = pid_file.with_extension("descendant");
        let descendant = Command::new(std::env::current_exe().expect("test executable exists"))
            .args(["process_group_child_helper", "--ignored", "--nocapture"])
            .env(GROUP_HELPER_ROLE_ENV, "descendant")
            .env(GROUP_HELPER_PID_FILE_ENV, &descendant_pid_file)
            .spawn()
            .expect("descendant should spawn");
        std::fs::write(
            &pid_file,
            format!("{} {}", std::process::id(), descendant.id()),
        )
        .expect("worker should write both PIDs");
        match std::env::var(GROUP_HELPER_MODE_ENV).as_deref() {
            Ok("exit") => return,
            Ok("failure") => std::process::exit(1),
            _ => {}
        }
        loop {
            thread::sleep(Duration::from_secs(1));
        }
    }

    #[test]
    fn process_group_exit_terminates_remaining_descendants() {
        let temp_dir = tempfile::tempdir().expect("fixture directory should be created");
        let pid_file = temp_dir.path().join("pids");
        let command = process_group_helper_command(&pid_file, "exit");

        let status = run_in_process_group_with_timeout(command, Duration::from_secs(5))
            .expect("helper should exit successfully");
        let pids = read_process_group_helper_pids(&pid_file);
        let alive = wait_for_process_group_to_exit(&pids);

        assert_eq!((status.success(), alive), (true, vec![false, false]),);
    }

    #[test]
    fn process_group_failure_terminates_remaining_descendants() {
        let temp_dir = tempfile::tempdir().expect("fixture directory should be created");
        let pid_file = temp_dir.path().join("pids");
        let command = process_group_helper_command(&pid_file, "failure");

        let status = run_in_process_group_with_timeout(command, Duration::from_secs(5))
            .expect("helper should exit with a failure status");
        let pids = read_process_group_helper_pids(&pid_file);
        let alive = wait_for_process_group_to_exit(&pids);

        assert_eq!((status.code(), alive), (Some(1), vec![false, false]),);
    }

    #[test]
    fn process_group_timeout_terminates_worker_and_descendants() {
        let temp_dir = tempfile::tempdir().expect("fixture directory should be created");
        let pid_file = temp_dir.path().join("pids");
        let command = process_group_helper_command(&pid_file, "wait");

        let result = run_in_process_group_with_timeout(command, Duration::from_secs(2));
        let pids = read_process_group_helper_pids(&pid_file);
        let alive = wait_for_process_group_to_exit(&pids);

        assert_eq!(
            (result.err().map(|error| error.kind()), alive),
            (Some(io::ErrorKind::TimedOut), vec![false, false]),
        );
    }

    #[test]
    fn process_group_interruption_terminates_worker_and_descendants() {
        let temp_dir = tempfile::tempdir().expect("fixture directory should be created");
        let pid_file = temp_dir.path().join("pids");
        let command = process_group_helper_command(&pid_file, "wait");
        let process_runner = thread::spawn(move || {
            run_in_process_group_with_timeout(command, Duration::from_secs(5))
        });
        let pids = read_process_group_helper_pids(&pid_file);
        GROUP_RUNNER_CANCEL_SIGNAL.store(libc::SIGTERM, Ordering::Relaxed);
        let result = process_runner
            .join()
            .expect("process group runner should return");
        let alive = wait_for_process_group_to_exit(&pids);

        assert_eq!(
            (result.err().map(|error| error.kind()), alive),
            (Some(io::ErrorKind::Interrupted), vec![false, false]),
        );
    }

    #[test]
    fn stderr_tail_reader_keeps_only_the_configured_suffix() {
        let actual = read_stderr_tail(
            &b"discard this; keep this suffix"[..],
            16,
            Arc::new(AtomicBool::new(false)),
        )
        .expect("fixture stderr should be readable");

        assert_eq!(actual, b"keep this suffix");
    }

    #[test]
    fn stderr_tail_reader_stops_after_cancellation_without_pipe_eof() {
        struct OpenPipe;

        impl io::Read for OpenPipe {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::ErrorKind::WouldBlock.into())
            }
        }

        let actual = read_stderr_tail(OpenPipe, 16, Arc::new(AtomicBool::new(true)))
            .expect("cancelled stderr reader should stop without EOF");

        assert_eq!(actual, Vec::<u8>::new());
    }

    #[test]
    fn process_group_runner_captures_only_the_configured_stderr_suffix() {
        let mut command = Command::new(std::env::current_exe().expect("test executable exists"));
        command
            .args(["process_group_child_helper", "--ignored", "--nocapture"])
            .env(GROUP_HELPER_ROLE_ENV, "stderr")
            .env(GROUP_HELPER_STDERR_ENV, "discard this; keep this suffix");

        let output =
            run_in_process_group_with_timeout_and_stderr(command, Duration::from_secs(5), 16)
                .expect("stderr fixture should exit successfully");

        assert_eq!(
            (
                output
                    .status
                    .expect("fixture should exit successfully")
                    .success(),
                output.stderr,
            ),
            (true, b"keep this suffix".to_vec())
        );
    }

    #[test]
    fn process_group_runner_preserves_stderr_when_timed_out() {
        let temp_dir = tempfile::tempdir().expect("fixture directory should be created");
        let ready_file = temp_dir.path().join("ready");
        let mut command = Command::new(std::env::current_exe().expect("test executable exists"));
        command
            .args(["process_group_child_helper", "--ignored", "--nocapture"])
            .env(GROUP_HELPER_ROLE_ENV, "stderr")
            .env(GROUP_HELPER_MODE_ENV, "wait")
            .env(GROUP_HELPER_READY_FILE_ENV, &ready_file)
            .env(GROUP_HELPER_STDERR_ENV, "discard this; keep this suffix");

        let process_runner = thread::spawn(move || {
            run_in_process_group_with_timeout_and_stderr(command, Duration::from_secs(5), 16)
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready_file.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let output = process_runner
            .join()
            .expect("process group runner should return")
            .expect("timed out fixture should return its process output");

        assert_eq!(
            (
                ready_file.exists(),
                output.status.err().map(|error| error.kind()),
                output.stderr,
            ),
            (
                true,
                Some(io::ErrorKind::TimedOut),
                b"keep this suffix".to_vec(),
            ),
        );
    }

    fn process_group_helper_command(pid_file: &Path, mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().expect("test executable exists"));
        command
            .args(["process_group_child_helper", "--ignored", "--nocapture"])
            .env(GROUP_HELPER_ROLE_ENV, "worker")
            .env(GROUP_HELPER_MODE_ENV, mode)
            .env(GROUP_HELPER_PID_FILE_ENV, pid_file);
        command
    }

    fn read_process_group_helper_pids(path: &Path) -> Vec<u32> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(contents) = std::fs::read_to_string(path) {
                let pids = contents
                    .split_whitespace()
                    .filter_map(|pid| pid.parse::<u32>().ok())
                    .collect::<Vec<_>>();
                if pids.len() == 2 {
                    return pids;
                }
            }
            assert!(
                Instant::now() < deadline,
                "process group helper did not report both PIDs"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_for_process_group_to_exit(pids: &[u32]) -> Vec<bool> {
        let deadline = Instant::now() + Duration::from_secs(2);
        while pids.iter().any(|pid| is_process_alive(*pid)) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        pids.iter().map(|pid| is_process_alive(*pid)).collect()
    }
}
