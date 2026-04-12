//! Thin abstraction over `kill(2)` so that tests can verify signal delivery
//! without actually killing processes.

use std::io;

/// Sends a Unix signal to a process.
///
/// The trait exists so `sweep` can be unit-tested against a fake
/// implementation that only records calls.
pub trait SignalSender {
    /// Sends `signal` (a raw signal number, e.g., `libc::SIGTERM`) to `pid`.
    /// Returns `Ok(())` on success. If the process does not exist the call
    /// returns `Err` with `io::ErrorKind::NotFound`.
    fn send(&self, pid: u32, signal: i32) -> io::Result<()>;
}

/// Production `SignalSender` that calls `libc::kill`.
pub struct LibcSignalSender;

impl SignalSender for LibcSignalSender {
    fn send(&self, pid: u32, signal: i32) -> io::Result<()> {
        // SAFETY: `kill(2)` takes a pid and a signal number. No memory is
        // shared with the process, the call is always safe regardless of
        // arguments (errors are reported via errno / return value).
        let ret = unsafe { libc::kill(pid as libc::pid_t, signal) };
        if ret == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::cell::RefCell;
    use std::io;

    use super::SignalSender;

    /// Test double that records every `send` call.
    #[derive(Default)]
    pub(crate) struct RecordingSender {
        pub calls: RefCell<Vec<(u32, i32)>>,
        /// When set, the next `send` call fails with ESRCH (process not found).
        pub fail_with_esrch: RefCell<bool>,
    }

    impl SignalSender for RecordingSender {
        fn send(&self, pid: u32, signal: i32) -> io::Result<()> {
            self.calls.borrow_mut().push((pid, signal));
            if self.fail_with_esrch.replace(false) {
                return Err(io::Error::from_raw_os_error(libc::ESRCH));
            }
            Ok(())
        }
    }
}
