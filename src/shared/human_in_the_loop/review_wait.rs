//! Event-driven wait for a review completion signal or its shpool shell to exit.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
use std::path::Path;
use std::time::{Duration, Instant};

use super::session_watch::{SessionResolution, SessionWatch};
use super::{HumanInTheLoopError, Result, try_read_fifo_byte};
use crate::infra::process::{FdWake, wait_for_exit_or_readable_fd};

const SESSION_CHECK_INTERVAL: Duration = Duration::from_secs(2);
const MAX_CONSECUTIVE_SESSION_CHECK_FAILURES: usize = 3;

pub(super) fn wait_for_fifo_signal_or_session_exit(
    file: File,
    fifo_path: &Path,
    session: &str,
    session_watch: &mut SessionWatch,
    resolve_shell_pid: impl FnMut(&str) -> Result<Option<u32>>,
) -> Result<u8> {
    wait_for_fifo_signal_or_session_exit_with_interval(
        file,
        fifo_path,
        session,
        session_watch,
        SESSION_CHECK_INTERVAL,
        resolve_shell_pid,
    )
}

fn wait_for_fifo_signal_or_session_exit_with_interval(
    mut file: File,
    fifo_path: &Path,
    session: &str,
    session_watch: &mut SessionWatch,
    check_interval: Duration,
    mut resolve_shell_pid: impl FnMut(&str) -> Result<Option<u32>>,
) -> Result<u8> {
    // Prevent a writer-less FIFO EOF from becoming a persistent readiness event during the review.
    let keep_writer = OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(fifo_path)?;
    if !keep_writer.metadata()?.file_type().is_fifo() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "review signal path is not a FIFO",
        )
        .into());
    }
    let _keep_writer = keep_writer;
    let fd = file.as_raw_fd();
    let mut next_session_check = Instant::now();
    let mut consecutive_check_failures = 0;

    let shell_pid = loop {
        if let Some(signal) = try_read_fifo_byte(&mut file)? {
            return Ok(signal);
        }

        if Instant::now() >= next_session_check {
            match session_watch.resolve(Instant::now(), || resolve_shell_pid(session)) {
                Ok(SessionResolution::Found(pid)) => {
                    break pid;
                }
                Ok(SessionResolution::Starting) => consecutive_check_failures = 0,
                Ok(SessionResolution::Closed) => {
                    return match try_read_fifo_byte(&mut file)? {
                        Some(signal) => Ok(signal),
                        None => Err(HumanInTheLoopError::ReviewSessionClosed),
                    };
                }
                Err(error) => {
                    consecutive_check_failures += 1;
                    tracing::warn!(
                        session,
                        consecutive_check_failures,
                        max_consecutive_failures = MAX_CONSECUTIVE_SESSION_CHECK_FAILURES,
                        error = %error,
                        "Review session liveness check failed"
                    );
                    if consecutive_check_failures >= MAX_CONSECUTIVE_SESSION_CHECK_FAILURES {
                        return Err(error);
                    }
                }
            }
            next_session_check = Instant::now() + check_interval;
        }

        let timeout = next_session_check.saturating_duration_since(Instant::now());
        match wait_for_exit_or_readable_fd(&[], fd, Some(timeout)) {
            FdWake::Readable => continue,
            FdWake::TimedOut => {}
            FdWake::Exited => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "unexpected wake source while resolving review session",
                )
                .into());
            }
        }
    };

    loop {
        match wait_for_exit_or_readable_fd(&[shell_pid], fd, None) {
            FdWake::Readable => {
                if let Some(signal) = try_read_fifo_byte(&mut file)? {
                    return Ok(signal);
                }
            }
            FdWake::Exited => {
                return match try_read_fifo_byte(&mut file)? {
                    Some(signal) => Ok(signal),
                    None => Err(HumanInTheLoopError::ReviewSessionClosed),
                };
            }
            FdWake::TimedOut => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::path::PathBuf;
    use std::thread;

    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::*;

    struct FifoReader {
        _temp: TempDir,
        path: PathBuf,
        reader: File,
    }

    struct PausedChild {
        pid: u32,
        reaped: bool,
    }

    impl PausedChild {
        fn spawn() -> Self {
            // SAFETY: the forked child only calls `pause` and `_exit`, which do not touch Rust
            // runtime state inherited from the multithreaded test process.
            let pid = unsafe { libc::fork() };
            assert!(pid >= 0, "child process should fork");
            if pid == 0 {
                // SAFETY: the child exits immediately if a signal interrupts `pause`.
                unsafe {
                    libc::pause();
                    libc::_exit(0);
                }
            }
            Self {
                pid: u32::try_from(pid).expect("child pid should fit in u32"),
                reaped: false,
            }
        }
    }

    impl Drop for PausedChild {
        fn drop(&mut self) {
            if !self.reaped {
                // SAFETY: this guard owns the child pid and reaps it if a test exits early.
                unsafe {
                    libc::kill(self.pid as libc::pid_t, libc::SIGKILL);
                    libc::waitpid(self.pid as libc::pid_t, std::ptr::null_mut(), 0);
                }
            }
        }
    }

    #[fixture]
    fn fifo_reader() -> FifoReader {
        let temp = TempDir::new().expect("temporary directory should be created");
        let document = temp.path().join("review.md");
        std::fs::write(&document, "").expect("document should be created");
        let path = super::super::create_fifo_with_suffix(&document, ".done")
            .expect("done fifo should be created");
        let reader = super::super::open_fifo_reader(&path).expect("fifo reader should open");
        FifoReader {
            _temp: temp,
            path,
            reader,
        }
    }

    fn signal_fifo(path: &Path, signal: u8) {
        let mut writer = OpenOptions::new()
            .write(true)
            .open(path)
            .expect("fifo writer should open");
        writer
            .write_all(&[signal])
            .expect("signal should be written");
    }

    fn session_watch() -> SessionWatch {
        SessionWatch::new()
    }

    #[rstest]
    fn returns_a_fifo_signal_while_the_session_is_alive(fifo_reader: FifoReader) {
        let path = fifo_reader.path.clone();
        let writer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(30));
            signal_fifo(&path, b'0');
        });

        let result = wait_for_fifo_signal_or_session_exit_with_interval(
            fifo_reader.reader,
            &fifo_reader.path,
            "review-session",
            &mut session_watch(),
            Duration::ZERO,
            |_| Ok(Some(std::process::id())),
        )
        .map_err(|error| error.to_string());
        writer.join().expect("writer should finish");

        assert_eq!(result, Ok(b'0'));
    }

    #[rstest]
    fn returns_session_closed_when_the_resolved_process_has_exited(fifo_reader: FifoReader) {
        let result = wait_for_fifo_signal_or_session_exit_with_interval(
            fifo_reader.reader,
            &fifo_reader.path,
            "review-session",
            &mut session_watch(),
            Duration::ZERO,
            |_| Ok(Some(u32::MAX)),
        )
        .map_err(|error| error.to_string());

        assert_eq!(
            result,
            Err("Review editor session ended before the review completed".to_string()),
        );
    }

    #[rstest]
    fn returns_session_closed_when_the_resolved_process_exits_during_the_wait(
        fifo_reader: FifoReader,
    ) {
        let child = PausedChild::spawn();
        let child_pid = child.pid;
        let releaser = thread::spawn(move || {
            thread::sleep(Duration::from_millis(250));
            // SAFETY: the child guard owns this pid and is waiting in `pause`.
            let killed = unsafe { libc::kill(child_pid as libc::pid_t, libc::SIGTERM) };
            let mut status = 0;
            // SAFETY: wait for the exact child sent the signal above.
            let reaped = unsafe { libc::waitpid(child_pid as libc::pid_t, &mut status, 0) };
            (killed, reaped)
        });

        let result = wait_for_fifo_signal_or_session_exit_with_interval(
            fifo_reader.reader,
            &fifo_reader.path,
            "review-session",
            &mut session_watch(),
            Duration::ZERO,
            |_| Ok(Some(child_pid)),
        )
        .map_err(|error| error.to_string());
        let reap_result = releaser.join();
        let mut child = child;
        child.reaped = true;
        let reap_result = reap_result.expect("child releaser should finish");

        assert_eq!(
            (result, reap_result),
            (
                Err("Review editor session ended before the review completed".to_string()),
                (0, child_pid as libc::pid_t),
            ),
        );
    }

    #[rstest]
    fn returns_a_signal_written_before_the_resolved_process_is_found_dead(fifo_reader: FifoReader) {
        let path = fifo_reader.path.clone();
        let result = wait_for_fifo_signal_or_session_exit_with_interval(
            fifo_reader.reader,
            &fifo_reader.path,
            "review-session",
            &mut session_watch(),
            Duration::ZERO,
            move |_| {
                signal_fifo(&path, b'0');
                Ok(Some(u32::MAX))
            },
        )
        .map_err(|error| error.to_string());

        assert_eq!(result, Ok(b'0'));
    }

    #[rstest]
    fn resolves_a_session_that_appears_after_the_wait_starts(fifo_reader: FifoReader) {
        let checks = Cell::new(0);
        let path = fifo_reader.path.clone();
        let writer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(70));
            signal_fifo(&path, b'0');
        });

        let result = wait_for_fifo_signal_or_session_exit_with_interval(
            fifo_reader.reader,
            &fifo_reader.path,
            "review-session",
            &mut session_watch(),
            Duration::from_millis(10),
            |_| {
                let check = checks.get() + 1;
                checks.set(check);
                Ok((check >= 2).then_some(std::process::id()))
            },
        )
        .map_err(|error| error.to_string());
        writer.join().expect("writer should finish");

        assert_eq!((result, checks.get()), (Ok(b'0'), 2));
    }

    #[rstest]
    fn waits_for_fifo_signal_without_reresolving_the_session(fifo_reader: FifoReader) {
        let checks = Cell::new(0);
        let path = fifo_reader.path.clone();
        let writer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(70));
            signal_fifo(&path, b'0');
        });

        let result = wait_for_fifo_signal_or_session_exit_with_interval(
            fifo_reader.reader,
            &fifo_reader.path,
            "review-session",
            &mut session_watch(),
            Duration::from_secs(2),
            |_| {
                checks.set(checks.get() + 1);
                Ok(None)
            },
        )
        .map_err(|error| error.to_string());
        writer.join().expect("writer should finish");

        assert_eq!((result, checks.get()), (Ok(b'0'), 1));
    }

    #[rstest]
    fn returns_the_third_consecutive_session_resolution_failure(fifo_reader: FifoReader) {
        let checks = Cell::new(0);
        let result = wait_for_fifo_signal_or_session_exit_with_interval(
            fifo_reader.reader,
            &fifo_reader.path,
            "review-session",
            &mut session_watch(),
            Duration::ZERO,
            |_| {
                checks.set(checks.get() + 1);
                Err(HumanInTheLoopError::CommandFailed(
                    "shpool unavailable".into(),
                ))
            },
        )
        .map_err(|error| error.to_string());

        assert_eq!(
            (result, checks.get()),
            (Err("Command failed: shpool unavailable".into()), 3),
        );
    }

    #[rstest]
    fn resets_session_resolution_failures_after_a_successful_lookup(fifo_reader: FifoReader) {
        let checks = Cell::new(0);
        let path = fifo_reader.path.clone();
        let writer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(70));
            signal_fifo(&path, b'0');
        });

        let result = wait_for_fifo_signal_or_session_exit_with_interval(
            fifo_reader.reader,
            &fifo_reader.path,
            "review-session",
            &mut session_watch(),
            Duration::ZERO,
            |_| {
                let check = checks.get() + 1;
                checks.set(check);
                match check {
                    1 | 3 | 4 => Err(HumanInTheLoopError::CommandFailed(
                        "shpool unavailable".into(),
                    )),
                    2 => Ok(None),
                    _ => Ok(Some(std::process::id())),
                }
            },
        )
        .map_err(|error| error.to_string());
        writer.join().expect("writer should finish");

        assert_eq!((result, checks.get()), (Ok(b'0'), 5));
    }
}
