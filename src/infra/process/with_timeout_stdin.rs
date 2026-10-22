use std::io::{self, Read, Write};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use super::{INITIAL_POLL_INTERVAL, MAX_POLL_INTERVAL};

pub(crate) fn run_with_timeout_stdin(
    mut command: Command,
    input: &[u8],
    timeout: Duration,
    output_limit: usize,
) -> io::Result<Output> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| io::Error::other("process timeout is out of range"))?;
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let (Some(mut stdin), Some(mut stdout), Some(mut stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(io::Error::other(
            "child spawned without piped standard streams",
        ));
    };

    let input = input.to_vec();
    let stdin_writer = thread::spawn(move || stdin.write_all(&input));
    let stdout_reader = thread::spawn(move || read_limited(&mut stdout, output_limit));
    let stderr_reader = thread::spawn(move || read_limited(&mut stderr, output_limit));

    let status = wait_for_exit(&mut child, deadline, timeout)?;
    let input_result = join_thread_until(stdin_writer, deadline, timeout, "stdin writer")?;
    let stdout = join_thread_until(stdout_reader, deadline, timeout, "stdout reader")??;
    let stderr = join_thread_until(stderr_reader, deadline, timeout, "stderr reader")??;
    let (stdout, stderr) = check_stream_limits(stdout, stderr, output_limit)?;
    input_result?;

    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

trait ChildControl {
    type Status;

    fn try_wait(&mut self) -> io::Result<Option<Self::Status>>;
    fn kill(&mut self) -> io::Result<()>;
    fn reap(&mut self) -> io::Result<()>;
}

impl ChildControl for Child {
    type Status = std::process::ExitStatus;

    fn try_wait(&mut self) -> io::Result<Option<Self::Status>> {
        Child::try_wait(self)
    }

    fn kill(&mut self) -> io::Result<()> {
        Child::kill(self)
    }

    fn reap(&mut self) -> io::Result<()> {
        self.wait().map(|_| ())
    }
}

fn wait_for_exit<Process>(
    child: &mut Process,
    deadline: Instant,
    timeout: Duration,
) -> io::Result<Process::Status>
where
    Process: ChildControl,
{
    let mut poll_interval = INITIAL_POLL_INTERVAL;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.reap();
                return Err(error);
            }
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.reap();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("process timed out after {timeout:?}"),
            ));
        }
        thread::sleep(poll_interval);
        poll_interval = (poll_interval * 2).min(MAX_POLL_INTERVAL);
    }
}

fn read_limited(reader: &mut impl Read, limit: usize) -> io::Result<(Vec<u8>, bool)> {
    let mut output = Vec::new();
    let mut chunk = [0; 8 * 1024];
    let mut truncated = false;
    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            return Ok((output, truncated));
        }
        let remaining = limit.saturating_sub(output.len());
        output.extend_from_slice(&chunk[..read.min(remaining)]);
        truncated |= read > remaining;
    }
}

fn check_stream_limits(
    stdout: (Vec<u8>, bool),
    stderr: (Vec<u8>, bool),
    limit: usize,
) -> io::Result<(Vec<u8>, Vec<u8>)> {
    if stdout.1 || stderr.1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("process output exceeds the {limit}-byte limit"),
        ));
    }
    Ok((stdout.0, stderr.0))
}

fn join_thread_until<T>(
    handle: thread::JoinHandle<T>,
    deadline: Instant,
    timeout: Duration,
    label: &str,
) -> io::Result<T> {
    while !handle.is_finished() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("process timed out after {timeout:?} while waiting for {label}"),
            ));
        }
        thread::sleep(Duration::from_millis(2).min(remaining));
    }
    handle
        .join()
        .map_err(|_| io::Error::other(format!("process {label} thread panicked")))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::io::{self, Cursor};
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use rstest::rstest;

    use super::{ChildControl, check_stream_limits, read_limited, wait_for_exit};

    struct FakeChild {
        exit_status: Option<()>,
        killed: Rc<Cell<bool>>,
        reaped: Rc<Cell<bool>>,
    }

    impl ChildControl for FakeChild {
        type Status = ();

        fn try_wait(&mut self) -> io::Result<Option<Self::Status>> {
            Ok(self.exit_status)
        }

        fn kill(&mut self) -> io::Result<()> {
            self.killed.set(true);
            Ok(())
        }

        fn reap(&mut self) -> io::Result<()> {
            self.reaped.set(true);
            Ok(())
        }
    }

    #[rstest]
    #[case::within_limit(b"small".to_vec(), 5, (b"small".to_vec(), false))]
    #[case::over_limit(b"longer".to_vec(), 5, (b"longe".to_vec(), true))]
    fn reads_only_the_configured_output_limit(
        #[case] input: Vec<u8>,
        #[case] limit: usize,
        #[case] expected: (Vec<u8>, bool),
    ) {
        let actual = read_limited(&mut Cursor::new(input), limit).expect("reader should succeed");

        assert_eq!(actual, expected);
    }

    #[rstest]
    #[case::within_limit(
        (b"ok".to_vec(), false),
        (b"".to_vec(), false),
        Ok((b"ok".to_vec(), Vec::new())),
    )]
    #[case::stdout_over_limit(
        (b"cap".to_vec(), true),
        (Vec::new(), false),
        Err(io::ErrorKind::InvalidData),
    )]
    #[case::stderr_over_limit(
        (Vec::new(), false),
        (b"cap".to_vec(), true),
        Err(io::ErrorKind::InvalidData),
    )]
    fn rejects_truncated_standard_streams(
        #[case] stdout: (Vec<u8>, bool),
        #[case] stderr: (Vec<u8>, bool),
        #[case] expected: std::result::Result<(Vec<u8>, Vec<u8>), io::ErrorKind>,
    ) {
        let actual = check_stream_limits(stdout, stderr, 2).map_err(|error| error.kind());

        assert_eq!(actual, expected);
    }

    #[test]
    fn kills_and_reaps_a_child_when_the_deadline_expires() {
        let killed = Rc::new(Cell::new(false));
        let reaped = Rc::new(Cell::new(false));
        let mut child = FakeChild {
            exit_status: None,
            killed: Rc::clone(&killed),
            reaped: Rc::clone(&reaped),
        };
        let result = wait_for_exit(
            &mut child,
            Instant::now() - Duration::from_millis(1),
            Duration::ZERO,
        )
        .map_err(|error| error.kind());

        assert_eq!(
            (result, killed.get(), reaped.get()),
            (Err(io::ErrorKind::TimedOut), true, true),
        );
    }

    #[test]
    fn returns_an_already_exited_child_without_killing_it() {
        let killed = Rc::new(Cell::new(false));
        let reaped = Rc::new(Cell::new(false));
        let mut child = FakeChild {
            exit_status: Some(()),
            killed: Rc::clone(&killed),
            reaped: Rc::clone(&reaped),
        };
        let result = wait_for_exit(
            &mut child,
            Instant::now() - Duration::from_millis(1),
            Duration::ZERO,
        );
        let result = result.map_err(|error| error.kind());

        assert_eq!((result, killed.get(), reaped.get()), (Ok(()), false, false));
    }
}
