//! Abstraction over asking an interactive agent to exit through its normal
//! terminal cleanup path.

use std::io;
use std::thread;
use std::time::{Duration, Instant};

use super::signal::LibcSignalSender;

const EXIT_GRACE_PERIOD: Duration = Duration::from_secs(5);
const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(25);

pub(crate) trait GracefulQuitRequester {
    /// Returns true when the process exits within the grace period.
    fn request_and_wait(&self, pane_id: &str, pid: u32) -> io::Result<bool>;
}

impl GracefulQuitRequester for LibcSignalSender {
    fn request_and_wait(&self, pane_id: &str, pid: u32) -> io::Result<bool> {
        crate::infra::tmux::run_tmux(&["send-keys", "-t", pane_id, "C-d"])
            .map_err(io::Error::other)?;

        let deadline = Instant::now() + EXIT_GRACE_PERIOD;
        loop {
            // SAFETY: signal 0 only checks whether the pid exists and does not
            // deliver a signal or access process memory.
            let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
            if result != 0 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            thread::sleep(EXIT_POLL_INTERVAL);
        }
    }
}

#[cfg(test)]
impl GracefulQuitRequester for super::signal::test_support::RecordingSender {
    fn request_and_wait(&self, pane_id: &str, _pid: u32) -> io::Result<bool> {
        self.ctrl_d_calls.borrow_mut().push(pane_id.to_string());
        if self.fail_ctrl_d.replace(false) {
            return Err(io::Error::other("tmux send-keys failed"));
        }
        Ok(*self.graceful_exit_result.borrow())
    }
}
