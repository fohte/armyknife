use std::collections::VecDeque;
use std::io::{ErrorKind, Read};
use std::process::{Child, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use super::super::CommandResult;
use crate::shared::command;

pub(super) const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const OUTPUT_TAIL_BYTES: usize = 8 * 1024;
// Detached descendants may keep inherited output pipes open after cleanup.
const OUTPUT_DRAIN_TIMEOUT: Duration = Duration::from_millis(100);
const PROCESS_CHECK_INTERVAL: Duration = Duration::from_millis(25);

pub(in crate::commands::agent::watch_web::actions) fn execute_command(
    cwd: &std::path::Path,
    command_text: &str,
) -> Result<CommandResult> {
    let shell = std::env::var_os("SHELL").context("SHELL is not set")?;
    let mut process = command::new(shell);
    process
        .arg("-lc")
        .arg(command_text)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        process.process_group(0);
    }
    let mut child = process.spawn().context("failed to start login shell")?;
    let stdout = child
        .stdout
        .take()
        .context("shell stdout was not captured")?;
    let stderr = child
        .stderr
        .take()
        .context("shell stderr was not captured")?;
    if let Err(error) = set_nonblocking(&stdout).and_then(|()| set_nonblocking(&stderr)) {
        terminate_process(&mut child);
        let _ = child.wait();
        return Err(error);
    }

    let stop_reading = Arc::new(AtomicBool::new(false));
    let stdout_stop_reading = Arc::clone(&stop_reading);
    let stderr_stop_reading = Arc::clone(&stop_reading);
    let stdout_reader = thread::spawn(move || {
        read_tail(
            stdout,
            OUTPUT_TAIL_BYTES,
            stdout_stop_reading,
            OUTPUT_DRAIN_TIMEOUT,
        )
    });
    let stderr_reader = thread::spawn(move || {
        read_tail(
            stderr,
            OUTPUT_TAIL_BYTES,
            stderr_stop_reading,
            OUTPUT_DRAIN_TIMEOUT,
        )
    });
    let (status, timed_out) = wait_for_process(&mut child, COMMAND_TIMEOUT)?;
    stop_reading.store(true, Ordering::Release);
    let stdout = stdout_reader
        .join()
        .map_err(|_| anyhow::anyhow!("stdout reader panicked"))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| anyhow::anyhow!("stderr reader panicked"))??;

    Ok(CommandResult {
        exit_code: if timed_out {
            124
        } else {
            status.code().unwrap_or(1)
        },
        stdout,
        stderr,
        timed_out,
    })
}

fn wait_for_process(
    child: &mut Child,
    timeout: Duration,
) -> Result<(std::process::ExitStatus, bool)> {
    #[cfg(unix)]
    {
        let process_id = match libc::pid_t::try_from(child.id()) {
            Ok(process_id) => process_id,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error).context("child PID is out of range");
            }
        };
        wait_for_completion(
            timeout,
            || child_has_exited_without_reaping(process_id).map_err(Into::into),
            || {
                terminate_process(child);
                child.wait().context("failed to stop login shell")
            },
        )
    }
    #[cfg(not(unix))]
    {
        let mut observed_status = None;
        wait_for_completion(
            timeout,
            || {
                observed_status = child.try_wait().context("failed to wait for login shell")?;
                Ok(observed_status.is_some())
            },
            || {
                if let Some(status) = observed_status {
                    Ok(status)
                } else {
                    terminate_process(child);
                    child.wait().context("failed to stop timed out login shell")
                }
            },
        )
    }
}

fn wait_for_completion<T>(
    timeout: Duration,
    mut has_exited: impl FnMut() -> Result<bool>,
    stop: impl FnOnce() -> Result<T>,
) -> Result<(T, bool)> {
    let deadline = Instant::now() + timeout;
    loop {
        match has_exited() {
            Ok(true) => return stop().map(|status| (status, false)),
            Ok(false) => {}
            Err(error) => {
                let _ = stop();
                return Err(error);
            }
        }
        if Instant::now() >= deadline {
            return stop().map(|status| (status, true));
        }
        thread::sleep(PROCESS_CHECK_INTERVAL);
    }
}

#[cfg(unix)]
fn child_has_exited_without_reaping(pid: libc::pid_t) -> std::io::Result<bool> {
    // WNOWAIT keeps the process-group ID from being reused before cleanup.
    // SAFETY: the structure is initialized by waitid before si_pid is read.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    // SAFETY: pid is the direct child and info is a writable siginfo_t.
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result == -1 {
        let error = std::io::Error::last_os_error();
        if error.kind() == ErrorKind::Interrupted {
            return Ok(false);
        }
        return Err(error);
    }
    // SAFETY: waitid initialized siginfo_t on success.
    Ok(unsafe { info.si_pid() } != 0)
}

fn terminate_process(child: &mut Child) {
    #[cfg(unix)]
    {
        let Ok(process_id) = libc::pid_t::try_from(child.id()) else {
            let _ = child.kill();
            return;
        };
        let process_group = -process_id;
        // The shell may have started child processes that still hold output pipes open.
        // SAFETY: the command was started in the process group matching its PID.
        let _ = unsafe { libc::kill(process_group, libc::SIGKILL) };
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
}

#[cfg(unix)]
fn set_nonblocking(reader: &impl std::os::fd::AsRawFd) -> Result<()> {
    let fd = reader.as_raw_fd();
    // SAFETY: F_GETFL only reads flags from the live pipe descriptor.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        return Err(std::io::Error::last_os_error()).context("failed to read pipe flags");
    }
    // SAFETY: F_SETFL updates flags on the live pipe descriptor.
    let result = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if result == -1 {
        return Err(std::io::Error::last_os_error())
            .context("failed to make output pipe nonblocking");
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_nonblocking(_: &impl Read) -> Result<()> {
    Ok(())
}

fn read_tail(
    mut reader: impl Read,
    limit: usize,
    stop_reading: Arc<AtomicBool>,
    drain_timeout: Duration,
) -> Result<String> {
    let mut tail = VecDeque::with_capacity(limit);
    let mut buffer = [0; 4096];
    let mut total_bytes = 0usize;
    let mut cancellation_deadline = None;
    let mut output_omitted = false;
    loop {
        let read = match reader.read(&mut buffer) {
            Ok(read) => read,
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                if stop_reading.load(Ordering::Acquire) {
                    break;
                }
                thread::sleep(PROCESS_CHECK_INTERVAL);
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        if read == 0 {
            break;
        }
        total_bytes = total_bytes.saturating_add(read);
        for byte in &buffer[..read] {
            if limit == 0 {
                continue;
            }
            if tail.len() == limit {
                tail.pop_front();
            }
            tail.push_back(*byte);
        }
        if stop_reading.load(Ordering::Acquire) {
            let deadline =
                *cancellation_deadline.get_or_insert_with(|| Instant::now() + drain_timeout);
            if Instant::now() >= deadline {
                output_omitted = true;
                break;
            }
        }
    }
    let truncated = total_bytes > limit || output_omitted;
    let output = String::from_utf8_lossy(&tail.into_iter().collect::<Vec<_>>()).into_owned();
    Ok(if truncated {
        format!("[earlier output omitted]\n{output}")
    } else {
        output
    })
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Error, Read};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    use rstest::rstest;

    use super::{read_tail, wait_for_completion};

    struct ReaderThatWouldBlock {
        has_emitted_data: bool,
    }

    struct RepeatingReader;

    impl Read for RepeatingReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            buffer[..3].copy_from_slice(b"out");
            Ok(3)
        }
    }

    impl Read for ReaderThatWouldBlock {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if self.has_emitted_data {
                return Err(Error::from(std::io::ErrorKind::WouldBlock));
            }
            self.has_emitted_data = true;
            buffer[..7].copy_from_slice(b"partial");
            Ok(7)
        }
    }

    #[test]
    fn stops_reading_when_a_pipe_would_block_after_process_cleanup() {
        let reader = ReaderThatWouldBlock {
            has_emitted_data: false,
        };
        let stop_reading = Arc::new(AtomicBool::new(true));

        assert_eq!(
            read_tail(reader, 32, stop_reading, Duration::from_millis(100),).unwrap(),
            "partial",
        );
    }

    #[test]
    fn stops_after_one_batch_when_cancellation_is_requested() {
        assert_eq!(
            read_tail(
                RepeatingReader,
                8,
                Arc::new(AtomicBool::new(true)),
                Duration::ZERO,
            )
            .unwrap(),
            "[earlier output omitted]\nout",
        );
    }

    #[rstest]
    #[case::within_limit(b"result", 8, "result")]
    #[case::exact_limit(b"result", 6, "result")]
    #[case::over_limit(b"first second", 6, "[earlier output omitted]\nsecond")]
    #[case::zero_limit(b"result", 0, "[earlier output omitted]\n")]
    fn keeps_only_the_tail_when_output_exceeds_limit(
        #[case] input: &[u8],
        #[case] limit: usize,
        #[case] expected: &str,
    ) {
        assert_eq!(
            read_tail(
                Cursor::new(input),
                limit,
                Arc::new(AtomicBool::new(false)),
                Duration::from_millis(100),
            )
            .unwrap(),
            expected,
        );
    }

    #[rstest]
    #[case::finished(true, false)]
    #[case::timed_out(false, true)]
    fn stops_after_completion_or_deadline(#[case] exited: bool, #[case] expected_timed_out: bool) {
        let mut stopped = false;
        let result = wait_for_completion(
            Duration::ZERO,
            || Ok(exited),
            || {
                stopped = true;
                Ok(23)
            },
        )
        .unwrap();

        assert_eq!((result, stopped), ((23, expected_timed_out), true));
    }
}
