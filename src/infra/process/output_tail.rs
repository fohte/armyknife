use std::collections::VecDeque;
use std::io::{self, ErrorKind, Read};
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

pub(crate) struct OutputTailOptions {
    pub(crate) max_bytes: usize,
    pub(crate) poll_interval: Duration,
    pub(crate) drain_timeout: Duration,
    pub(crate) drain_deadline_check: DrainDeadlineCheck,
    pub(crate) retry_interrupted: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DrainDeadlineCheck {
    BeforeRead,
    AfterRead,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct OutputTail {
    pub(crate) bytes: Vec<u8>,
    pub(crate) truncated: bool,
}

#[cfg(unix)]
pub(crate) fn set_nonblocking(reader: &impl AsRawFd) -> io::Result<()> {
    let descriptor = reader.as_raw_fd();
    // SAFETY: F_GETFL only reads flags from the live pipe descriptor.
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: F_SETFL updates flags on the live pipe descriptor.
    let result = unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if result == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn set_nonblocking(_: &impl Read) -> io::Result<()> {
    Ok(())
}

pub(crate) fn read_output_tail(
    mut reader: impl Read,
    cancelled: Arc<AtomicBool>,
    options: OutputTailOptions,
) -> io::Result<OutputTail> {
    let mut tail = VecDeque::with_capacity(options.max_bytes);
    let mut buffer = [0; 4096];
    let mut total_bytes = 0usize;
    let mut drain_deadline = None;
    let mut drain_timed_out = false;

    loop {
        if options.drain_deadline_check == DrainDeadlineCheck::BeforeRead
            && cancelled.load(Ordering::Acquire)
        {
            let deadline =
                *drain_deadline.get_or_insert_with(|| Instant::now() + options.drain_timeout);
            if Instant::now() >= deadline {
                drain_timed_out = true;
                break;
            }
        }

        let read = match reader.read(&mut buffer) {
            Ok(read) => read,
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                if cancelled.load(Ordering::Acquire) {
                    break;
                }
                thread::sleep(options.poll_interval);
                continue;
            }
            Err(error) if error.kind() == ErrorKind::Interrupted && options.retry_interrupted => {
                continue;
            }
            Err(error) => return Err(error),
        };
        if read == 0 {
            break;
        }

        total_bytes = total_bytes.saturating_add(read);
        for byte in &buffer[..read] {
            if options.max_bytes == 0 {
                continue;
            }
            if tail.len() == options.max_bytes {
                tail.pop_front();
            }
            tail.push_back(*byte);
        }

        if options.drain_deadline_check == DrainDeadlineCheck::AfterRead
            && cancelled.load(Ordering::Acquire)
        {
            let deadline =
                *drain_deadline.get_or_insert_with(|| Instant::now() + options.drain_timeout);
            if Instant::now() >= deadline {
                drain_timed_out = true;
                break;
            }
        }
    }

    Ok(OutputTail {
        bytes: tail.into_iter().collect(),
        truncated: total_bytes > options.max_bytes || drain_timed_out,
    })
}

#[cfg(test)]
mod tests {
    use std::io::{self, Cursor, Error, Read};
    use std::os::fd::AsRawFd;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    use rstest::rstest;

    use super::{
        DrainDeadlineCheck, OutputTail, OutputTailOptions, read_output_tail, set_nonblocking,
    };

    struct ReaderThatWouldBlock {
        has_emitted_data: bool,
    }

    struct RepeatingReader;

    struct OpenPipe;

    impl Read for ReaderThatWouldBlock {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.has_emitted_data {
                return Err(Error::from(io::ErrorKind::WouldBlock));
            }
            self.has_emitted_data = true;
            buffer[..7].copy_from_slice(b"partial");
            Ok(7)
        }
    }

    impl Read for RepeatingReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            buffer[..3].copy_from_slice(b"out");
            Ok(3)
        }
    }

    impl Read for OpenPipe {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::ErrorKind::WouldBlock.into())
        }
    }

    fn options(
        max_bytes: usize,
        drain_timeout: Duration,
        drain_deadline_check: DrainDeadlineCheck,
        retry_interrupted: bool,
    ) -> OutputTailOptions {
        OutputTailOptions {
            max_bytes,
            poll_interval: Duration::from_millis(1),
            drain_timeout,
            drain_deadline_check,
            retry_interrupted,
        }
    }

    #[rstest]
    #[case::within_limit(b"result", 8, b"result", false)]
    #[case::exact_limit(b"result", 6, b"result", false)]
    #[case::over_limit(b"first second", 6, b"second", true)]
    #[case::zero_limit(b"result", 0, b"", true)]
    fn retains_only_the_configured_suffix(
        #[case] input: &[u8],
        #[case] max_bytes: usize,
        #[case] expected_bytes: &[u8],
        #[case] expected_truncated: bool,
    ) {
        let actual = read_output_tail(
            Cursor::new(input),
            Arc::new(AtomicBool::new(false)),
            options(
                max_bytes,
                Duration::from_millis(100),
                DrainDeadlineCheck::AfterRead,
                false,
            ),
        )
        .expect("fixture output should be readable");

        assert_eq!(
            actual,
            OutputTail {
                bytes: expected_bytes.to_vec(),
                truncated: expected_truncated,
            },
        );
    }

    #[test]
    fn drains_available_bytes_then_stops_when_a_cancelled_pipe_would_block() {
        let actual = read_output_tail(
            ReaderThatWouldBlock {
                has_emitted_data: false,
            },
            Arc::new(AtomicBool::new(true)),
            options(
                32,
                Duration::from_millis(100),
                DrainDeadlineCheck::AfterRead,
                false,
            ),
        )
        .expect("fixture output should be readable");

        assert_eq!(
            actual,
            OutputTail {
                bytes: b"partial".to_vec(),
                truncated: false,
            },
        );
    }

    #[test]
    fn reports_truncation_when_drain_timeout_expires_while_output_continues() {
        let actual = read_output_tail(
            RepeatingReader,
            Arc::new(AtomicBool::new(true)),
            options(8, Duration::ZERO, DrainDeadlineCheck::AfterRead, false),
        )
        .expect("fixture output should be readable");

        assert_eq!(
            actual,
            OutputTail {
                bytes: b"out".to_vec(),
                truncated: true,
            },
        );
    }

    #[test]
    fn stops_when_a_cancelled_pipe_would_block_without_eof() {
        let actual = read_output_tail(
            OpenPipe,
            Arc::new(AtomicBool::new(true)),
            options(
                16,
                Duration::from_millis(100),
                DrainDeadlineCheck::BeforeRead,
                true,
            ),
        )
        .expect("cancelled output reader should stop without EOF");

        assert_eq!(
            actual,
            OutputTail {
                bytes: Vec::new(),
                truncated: false,
            },
        );
    }

    #[test]
    fn does_not_read_after_a_process_group_drain_deadline() {
        let actual = read_output_tail(
            RepeatingReader,
            Arc::new(AtomicBool::new(true)),
            options(8, Duration::ZERO, DrainDeadlineCheck::BeforeRead, true),
        )
        .expect("fixture output should be readable");

        assert_eq!(
            actual,
            OutputTail {
                bytes: Vec::new(),
                truncated: true,
            },
        );
    }

    #[cfg(unix)]
    #[test]
    fn makes_output_descriptors_nonblocking() {
        let (reader, _writer) =
            std::os::unix::net::UnixStream::pair().expect("local socket pair should be created");

        set_nonblocking(&reader).expect("output descriptor should become nonblocking");
        // SAFETY: F_GETFL only reads flags from the live socket descriptor.
        let flags = unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_GETFL) };

        assert_eq!(flags & libc::O_NONBLOCK, libc::O_NONBLOCK);
    }
}
