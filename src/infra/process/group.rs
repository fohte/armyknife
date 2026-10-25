use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use super::{INITIAL_POLL_INTERVAL, MAX_POLL_INTERVAL};

const PROCESS_GROUP_TERMINATION_GRACE: Duration = Duration::from_millis(250);

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
    mut command: Command,
    timeout: Duration,
) -> io::Result<ExitStatus> {
    let _runner_lock = GROUP_RUNNER_LOCK
        .lock()
        .map_err(|_| io::Error::other("process-group runner lock was poisoned"))?;
    GROUP_RUNNER_CANCEL_SIGNAL.store(0, Ordering::Relaxed);
    let _signal_guard = install_group_runner_signal_handlers()?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
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
    let deadline = Instant::now() + timeout;
    let mut poll_interval = INITIAL_POLL_INTERVAL;

    loop {
        if GROUP_RUNNER_CANCEL_SIGNAL.load(Ordering::Relaxed) != 0 {
            process_group.terminate();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "command interrupted",
            ));
        }

        match child_has_exited_without_reaping(pid) {
            Ok(true) => {
                process_group.terminate();
                return child.wait();
            }
            Ok(false) => {}
            Err(error) => {
                process_group.terminate();
                let _ = child.wait();
                return Err(error);
            }
        }

        if Instant::now() >= deadline {
            process_group.terminate();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("command timed out after {timeout:?}"),
            ));
        }

        thread::sleep(poll_interval);
        poll_interval = std::cmp::min(poll_interval * 2, MAX_POLL_INTERVAL);
    }
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
        if std::env::var(GROUP_HELPER_MODE_ENV).is_ok_and(|mode| mode == "exit") {
            return;
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
        let deadline = Instant::now() + Duration::from_secs(3);
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
