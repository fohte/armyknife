//! Blocking wait for process exit, event-driven where the OS allows it.

use std::thread;
use std::time::{Duration, Instant};

use super::is_process_alive;

/// How late the `kill(pid, 0)` fallback may notice an exit. Coarse on purpose:
/// callers that need prompt wake-ups run on macOS, where kqueue is used.
const FALLBACK_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Blocks until at least one of `pids` has exited and returns `true`, or
/// returns `false` once `timeout` elapses (`None` waits indefinitely).
///
/// A pid that is already gone counts as exited, so a process that dies before
/// the wait starts is never missed. With an empty `pids` this only returns at
/// the timeout.
pub(crate) fn wait_for_any_exit(pids: &[u32], timeout: Option<Duration>) -> bool {
    #[cfg(target_os = "macos")]
    {
        // kqueue can fail (descriptor exhaustion, a pid we may not observe);
        // polling reaches the same answer later, so it is a safe fallback.
        if let Ok(exited) = wait_with_kqueue(pids, timeout) {
            return exited;
        }
    }
    poll_for_any_exit(pids, timeout)
}

fn poll_for_any_exit(pids: &[u32], timeout: Option<Duration>) -> bool {
    let deadline = timeout.map(|timeout| Instant::now() + timeout);
    loop {
        if pids.iter().any(|&pid| !is_process_alive(pid)) {
            return true;
        }
        let sleep = match deadline {
            Some(deadline) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return false;
                }
                remaining.min(FALLBACK_POLL_INTERVAL)
            }
            None => FALLBACK_POLL_INTERVAL,
        };
        thread::sleep(sleep);
    }
}

#[cfg(target_os = "macos")]
fn wait_with_kqueue(pids: &[u32], timeout: Option<Duration>) -> std::io::Result<bool> {
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::ptr;

    // SAFETY: `kqueue` takes no arguments and returns a new descriptor or -1.
    let raw = unsafe { libc::kqueue() };
    if raw == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `raw` was just created and nothing else owns it; `OwnedFd` closes it.
    let kq = unsafe { OwnedFd::from_raw_fd(raw) };

    for &pid in pids {
        // Mirrors `is_process_alive`: an unrepresentable or zero pid is not a live process.
        if pid == 0 || libc::pid_t::try_from(pid).is_err() {
            return Ok(true);
        }
        let change = libc::kevent {
            ident: pid as libc::uintptr_t,
            filter: libc::EVFILT_PROC,
            flags: libc::EV_ADD | libc::EV_ONESHOT,
            fflags: libc::NOTE_EXIT,
            data: 0,
            udata: ptr::null_mut(),
        };
        // SAFETY: `change` is a valid one-element changelist and no eventlist is requested.
        let result =
            unsafe { libc::kevent(kq.as_raw_fd(), &change, 1, ptr::null_mut(), 0, ptr::null()) };
        if result == -1 {
            let error = io::Error::last_os_error();
            // ESRCH means the process exited before it could be registered.
            return if error.raw_os_error() == Some(libc::ESRCH) {
                Ok(true)
            } else {
                Err(error)
            };
        }
    }

    let deadline = timeout.map(|timeout| Instant::now() + timeout);
    loop {
        let timespec = deadline.map(|deadline| {
            let remaining = deadline.saturating_duration_since(Instant::now());
            libc::timespec {
                tv_sec: libc::time_t::try_from(remaining.as_secs()).unwrap_or(libc::time_t::MAX),
                tv_nsec: libc::c_long::from(remaining.subsec_nanos()),
            }
        });
        let mut event = libc::kevent {
            ident: 0,
            filter: 0,
            flags: 0,
            fflags: 0,
            data: 0,
            udata: ptr::null_mut(),
        };
        // SAFETY: `event` has room for the single event requested, and `timespec`
        // outlives the call.
        let ready = unsafe {
            libc::kevent(
                kq.as_raw_fd(),
                ptr::null(),
                0,
                &mut event,
                1,
                timespec.as_ref().map_or(ptr::null(), ptr::from_ref),
            )
        };
        match ready {
            -1 => {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error);
                }
            }
            0 => return Ok(false),
            _ => return Ok(true),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::process::{Child, ChildStdin, Command, Stdio};
    use std::thread::JoinHandle;

    use rstest::rstest;

    use super::*;

    type Wait = fn(&[u32], Option<Duration>) -> bool;

    #[cfg(target_os = "macos")]
    fn wait_with_kqueue_only(pids: &[u32], timeout: Option<Duration>) -> bool {
        wait_with_kqueue(pids, timeout).expect("kqueue should be usable in tests")
    }

    /// A `sh` child that exits once its stdin closes. A thread reaps it so the
    /// pid disappears like a normal process's instead of lingering as a zombie,
    /// which `kill(pid, 0)` still reports alive.
    struct Holdable {
        pid: u32,
        stdin: Option<ChildStdin>,
        reaper: Option<JoinHandle<io::Result<()>>>,
    }

    impl Holdable {
        // `sh` is the POSIX-guaranteed primitive here: only a real process can
        // exercise the OS exit notification, and there is no logic to mock.
        fn spawn() -> Self {
            let mut child: Child = Command::new("sh")
                .args(["-c", "read _"])
                .stdin(Stdio::piped())
                .spawn()
                .expect("sh should spawn");
            let pid = child.id();
            let stdin = child.stdin.take();
            Self {
                pid,
                stdin,
                reaper: Some(thread::spawn(move || child.wait().map(|_| ()))),
            }
        }

        fn release(&mut self) {
            drop(self.stdin.take());
            if let Some(reaper) = self.reaper.take() {
                reaper
                    .join()
                    .expect("reaper should not panic")
                    .expect("child should be reaped");
            }
        }
    }

    impl Drop for Holdable {
        fn drop(&mut self) {
            self.release();
        }
    }

    #[rstest]
    #[case::default(wait_for_any_exit as Wait)]
    #[case::polling(poll_for_any_exit as Wait)]
    #[cfg_attr(target_os = "macos", case::kqueue(wait_with_kqueue_only as Wait))]
    fn returns_false_when_every_pid_outlives_the_timeout(#[case] wait: Wait) {
        let held = Holdable::spawn();

        assert!(!wait(&[held.pid], Some(Duration::from_millis(100))));
    }

    #[rstest]
    #[case::default(wait_for_any_exit as Wait)]
    #[case::polling(poll_for_any_exit as Wait)]
    #[cfg_attr(target_os = "macos", case::kqueue(wait_with_kqueue_only as Wait))]
    fn returns_when_one_of_several_pids_exits_and_leaves_the_others_alone(#[case] wait: Wait) {
        let survivor = Holdable::spawn();
        let mut leaver = Holdable::spawn();
        let pids = [survivor.pid, leaver.pid];
        let releaser = thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            leaver.release();
        });

        let exited = wait(&pids, Some(Duration::from_secs(30)));
        releaser.join().expect("releaser should not panic");

        assert_eq!((exited, is_process_alive(survivor.pid)), (true, true));
    }

    #[rstest]
    #[case::default(wait_for_any_exit as Wait)]
    #[case::polling(poll_for_any_exit as Wait)]
    #[cfg_attr(target_os = "macos", case::kqueue(wait_with_kqueue_only as Wait))]
    fn returns_immediately_for_a_pid_that_is_already_gone(#[case] wait: Wait) {
        let mut gone = Holdable::spawn();
        gone.release();

        assert!(wait(&[gone.pid], Some(Duration::from_secs(30))));
    }

    #[rstest]
    #[case::default(wait_for_any_exit as Wait)]
    #[case::polling(poll_for_any_exit as Wait)]
    #[cfg_attr(target_os = "macos", case::kqueue(wait_with_kqueue_only as Wait))]
    fn returns_immediately_for_a_pid_that_cannot_be_a_process(
        #[case] wait: Wait,
        #[values(0, u32::MAX)] pid: u32,
    ) {
        assert!(wait(&[pid], Some(Duration::from_secs(30))));
    }
}
