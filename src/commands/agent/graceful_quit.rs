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

    /// Returns true when the process exits after a request sent to its recorded server.
    fn request_and_wait_for_socket(
        &self,
        socket_path: Option<&str>,
        pane_id: &str,
        pid: u32,
    ) -> io::Result<bool> {
        if socket_path.is_some() {
            return Err(io::Error::other(
                "graceful quit requester does not support tmux sockets",
            ));
        }
        self.request_and_wait(pane_id, pid)
    }
}

impl GracefulQuitRequester for LibcSignalSender {
    fn request_and_wait(&self, pane_id: &str, pid: u32) -> io::Result<bool> {
        self.request_and_wait_for_socket(None, pane_id, pid)
    }

    fn request_and_wait_for_socket(
        &self,
        socket_path: Option<&str>,
        pane_id: &str,
        pid: u32,
    ) -> io::Result<bool> {
        crate::infra::tmux::send_keys_for_socket(socket_path, pane_id, "C-d")
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
        self.request_and_wait_for_socket(None, pane_id, _pid)
    }

    fn request_and_wait_for_socket(
        &self,
        socket_path: Option<&str>,
        pane_id: &str,
        _pid: u32,
    ) -> io::Result<bool> {
        self.ctrl_d_calls.borrow_mut().push(pane_id.to_string());
        self.ctrl_d_socket_calls
            .borrow_mut()
            .push((socket_path.map(str::to_string), pane_id.to_string()));
        if self.fail_ctrl_d.replace(false) {
            return Err(io::Error::other("tmux send-keys failed"));
        }
        Ok(*self.graceful_exit_result.borrow())
    }
}
