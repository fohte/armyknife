//! Blocking wait for a process exit or a directory change, event-driven where the OS allows it.

use std::fs;
use std::io;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use super::is_process_alive;

/// How late the polling fallback may notice an event. Coarse on purpose:
/// callers that need prompt wake-ups run on macOS, where kqueue is used.
const FALLBACK_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// What ended a [`wait_for_exit_or_dir_change`].
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Wake {
    /// These pids exited. A pid that is already gone counts as exited, so a process that
    /// dies before the wait starts is never missed.
    Exited(Vec<u32>),
    /// The entry names of the directory no longer equal the ones the caller knew.
    DirChanged,
    TimedOut,
}

/// Sorted entry names of `dir`; a missing directory has none.
pub(crate) fn dir_entry_names(dir: &Path) -> io::Result<Vec<String>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut names = entries
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<io::Result<Vec<_>>>()?;
    names.sort();
    Ok(names)
}

/// Blocks until one of `pids` exits, the entries of `dir` stop matching `known_entries` (as
/// listed by [`dir_entry_names`]), or `timeout` elapses (`None` waits indefinitely).
///
/// `known_entries` is what the caller last looked at, so a change between that look and this
/// call is reported instead of lost. `dir` should exist: without it the wait is not event-driven.
pub(crate) fn wait_for_exit_or_dir_change(
    pids: &[u32],
    dir: &Path,
    known_entries: &[String],
    timeout: Option<Duration>,
) -> Wake {
    #[cfg(target_os = "macos")]
    {
        // kqueue can fail (descriptor exhaustion, a pid we may not observe, a directory we
        // cannot open); polling reaches the same answer later, so it is a safe fallback.
        if let Ok(wake) = wait_with_kqueue(pids, dir, known_entries, timeout) {
            return wake;
        }
    }
    poll_for_exit_or_dir_change(pids, dir, known_entries, timeout)
}

/// An unreadable directory never counts as changed, so it cannot wake the caller in a loop.
fn dir_changed(dir: &Path, known_entries: &[String]) -> bool {
    dir_entry_names(dir).is_ok_and(|names| names != known_entries)
}

fn poll_for_exit_or_dir_change(
    pids: &[u32],
    dir: &Path,
    known_entries: &[String],
    timeout: Option<Duration>,
) -> Wake {
    let deadline = timeout.map(|timeout| Instant::now() + timeout);
    loop {
        let exited = pids
            .iter()
            .copied()
            .filter(|&pid| !is_process_alive(pid))
            .collect::<Vec<_>>();
        if !exited.is_empty() {
            return Wake::Exited(exited);
        }
        if dir_changed(dir, known_entries) {
            return Wake::DirChanged;
        }
        let sleep = match deadline {
            Some(deadline) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Wake::TimedOut;
                }
                remaining.min(FALLBACK_POLL_INTERVAL)
            }
            None => FALLBACK_POLL_INTERVAL,
        };
        thread::sleep(sleep);
    }
}

#[cfg(target_os = "macos")]
fn new_kevent(ident: libc::uintptr_t, filter: i16, flags: u16, fflags: u32) -> libc::kevent {
    libc::kevent {
        ident,
        filter,
        flags,
        fflags,
        data: 0,
        udata: std::ptr::null_mut(),
    }
}

#[cfg(target_os = "macos")]
fn register_kevent(kq: std::os::fd::RawFd, change: &libc::kevent) -> io::Result<()> {
    // SAFETY: `change` is a valid one-element changelist and no eventlist is requested.
    let result = unsafe { libc::kevent(kq, change, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
    if result == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn wait_with_kqueue(
    pids: &[u32],
    dir: &Path,
    known_entries: &[String],
    timeout: Option<Duration>,
) -> io::Result<Wake> {
    use std::fs::File;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::ptr;

    // SAFETY: `kqueue` takes no arguments and returns a new descriptor or -1.
    let raw = unsafe { libc::kqueue() };
    if raw == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `raw` was just created and nothing else owns it; `OwnedFd` closes it.
    let kq = unsafe { OwnedFd::from_raw_fd(raw) };

    // The directory is registered before `known_entries` is compared below, so an entry added
    // after the caller listed it either shows up in that comparison or fires this watch.
    // Closing the handle on return drops the registration.
    let dir_handle = File::open(dir)?;
    register_kevent(
        kq.as_raw_fd(),
        &new_kevent(
            dir_handle.as_raw_fd() as libc::uintptr_t,
            libc::EVFILT_VNODE,
            libc::EV_ADD,
            libc::NOTE_WRITE | libc::NOTE_DELETE | libc::NOTE_RENAME | libc::NOTE_REVOKE,
        ),
    )?;

    let mut exited = Vec::new();
    for &pid in pids {
        // Mirrors `is_process_alive`: an unrepresentable or zero pid is not a live process.
        if pid == 0 || libc::pid_t::try_from(pid).is_err() {
            exited.push(pid);
            continue;
        }
        let change = new_kevent(
            pid as libc::uintptr_t,
            libc::EVFILT_PROC,
            libc::EV_ADD | libc::EV_ONESHOT,
            libc::NOTE_EXIT,
        );
        match register_kevent(kq.as_raw_fd(), &change) {
            Ok(()) => {}
            // ESRCH means the process exited before it could be registered.
            Err(error) if error.raw_os_error() == Some(libc::ESRCH) => exited.push(pid),
            Err(error) => return Err(error),
        }
    }
    if !exited.is_empty() {
        return Ok(Wake::Exited(exited));
    }
    if dir_changed(dir, known_entries) {
        return Ok(Wake::DirChanged);
    }

    // Room for one event per pid plus the directory, so a batch of exits is reported together.
    let mut events = vec![new_kevent(0, 0, 0, 0); pids.len() + 1];
    let capacity = libc::c_int::try_from(events.len()).unwrap_or(libc::c_int::MAX);
    let deadline = timeout.map(|timeout| Instant::now() + timeout);
    loop {
        let timespec = deadline.map(|deadline| {
            let remaining = deadline.saturating_duration_since(Instant::now());
            libc::timespec {
                tv_sec: libc::time_t::try_from(remaining.as_secs()).unwrap_or(libc::time_t::MAX),
                tv_nsec: libc::c_long::from(remaining.subsec_nanos()),
            }
        });
        // SAFETY: `events` has room for `capacity` events, and `timespec` outlives the call.
        let ready = unsafe {
            libc::kevent(
                kq.as_raw_fd(),
                ptr::null(),
                0,
                events.as_mut_ptr(),
                capacity,
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
            0 => return Ok(Wake::TimedOut),
            ready => {
                let exited = events
                    .iter()
                    .take(ready as usize)
                    .filter(|event| event.filter == libc::EVFILT_PROC)
                    .map(|event| event.ident as u32)
                    .collect::<Vec<_>>();
                // Only the directory watch is left when no process exited.
                return Ok(if exited.is_empty() {
                    Wake::DirChanged
                } else {
                    Wake::Exited(exited)
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::process::{Child, ChildStdin, Command, Stdio};
    use std::thread::JoinHandle;

    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::*;

    type Wait = fn(&[u32], &Path, &[String], Option<Duration>) -> Wake;

    #[cfg(target_os = "macos")]
    fn wait_with_kqueue_only(
        pids: &[u32],
        dir: &Path,
        known_entries: &[String],
        timeout: Option<Duration>,
    ) -> Wake {
        wait_with_kqueue(pids, dir, known_entries, timeout)
            .expect("kqueue should be usable in tests")
    }

    #[fixture]
    fn dir() -> TempDir {
        TempDir::new().expect("temporary directory should be created")
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
        // Spawns a real process: only an actual exit can trigger the OS exit notification under test.
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
    #[case::default(wait_for_exit_or_dir_change as Wait)]
    #[case::polling(poll_for_exit_or_dir_change as Wait)]
    #[cfg_attr(target_os = "macos", case::kqueue(wait_with_kqueue_only as Wait))]
    fn times_out_when_nothing_happens(#[case] wait: Wait, dir: TempDir) {
        let held = Holdable::spawn();

        assert_eq!(
            wait(
                &[held.pid],
                dir.path(),
                &[],
                Some(Duration::from_millis(100))
            ),
            Wake::TimedOut,
        );
    }

    #[rstest]
    #[case::default(wait_for_exit_or_dir_change as Wait)]
    #[case::polling(poll_for_exit_or_dir_change as Wait)]
    #[cfg_attr(target_os = "macos", case::kqueue(wait_with_kqueue_only as Wait))]
    fn reports_the_pid_that_exits_and_leaves_the_others_alone(#[case] wait: Wait, dir: TempDir) {
        let survivor = Holdable::spawn();
        let mut leaver = Holdable::spawn();
        let (survivor_pid, leaver_pid) = (survivor.pid, leaver.pid);
        let releaser = thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            leaver.release();
        });

        let wake = wait(
            &[survivor_pid, leaver_pid],
            dir.path(),
            &[],
            Some(Duration::from_secs(30)),
        );
        releaser.join().expect("releaser should not panic");

        assert_eq!(
            (wake, is_process_alive(survivor_pid)),
            (Wake::Exited(vec![leaver_pid]), true),
        );
    }

    #[rstest]
    #[case::default(wait_for_exit_or_dir_change as Wait)]
    #[case::polling(poll_for_exit_or_dir_change as Wait)]
    #[cfg_attr(target_os = "macos", case::kqueue(wait_with_kqueue_only as Wait))]
    fn returns_immediately_for_a_pid_that_is_already_gone(#[case] wait: Wait, dir: TempDir) {
        let mut gone = Holdable::spawn();
        gone.release();

        assert_eq!(
            wait(&[gone.pid], dir.path(), &[], Some(Duration::from_secs(30))),
            Wake::Exited(vec![gone.pid]),
        );
    }

    #[rstest]
    #[case::default(wait_for_exit_or_dir_change as Wait)]
    #[case::polling(poll_for_exit_or_dir_change as Wait)]
    #[cfg_attr(target_os = "macos", case::kqueue(wait_with_kqueue_only as Wait))]
    fn returns_immediately_for_a_pid_that_cannot_be_a_process(
        #[case] wait: Wait,
        #[values(0, u32::MAX)] pid: u32,
        dir: TempDir,
    ) {
        assert_eq!(
            wait(&[pid], dir.path(), &[], Some(Duration::from_secs(30))),
            Wake::Exited(vec![pid]),
        );
    }

    #[rstest]
    #[case::default(wait_for_exit_or_dir_change as Wait)]
    #[case::polling(poll_for_exit_or_dir_change as Wait)]
    #[cfg_attr(target_os = "macos", case::kqueue(wait_with_kqueue_only as Wait))]
    fn wakes_when_an_entry_is_added_while_waiting(#[case] wait: Wait, dir: TempDir) {
        let held = Holdable::spawn();
        let new_entry = dir.path().join("4242");
        let adder = thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            fs::write(new_entry, "").expect("entry should be written");
        });

        let wake = wait(&[held.pid], dir.path(), &[], Some(Duration::from_secs(30)));
        adder.join().expect("adder should not panic");

        assert_eq!(wake, Wake::DirChanged);
    }

    #[rstest]
    #[case::default(wait_for_exit_or_dir_change as Wait)]
    #[case::polling(poll_for_exit_or_dir_change as Wait)]
    #[cfg_attr(target_os = "macos", case::kqueue(wait_with_kqueue_only as Wait))]
    fn reports_a_change_made_before_the_wait_started(#[case] wait: Wait, dir: TempDir) {
        let held = Holdable::spawn();
        fs::write(dir.path().join("4242"), "").expect("entry should be written");

        assert_eq!(
            wait(&[held.pid], dir.path(), &[], Some(Duration::from_secs(30))),
            Wake::DirChanged,
        );
    }

    #[rstest]
    fn lists_entry_names_sorted_and_treats_a_missing_directory_as_empty(dir: TempDir) {
        fs::write(dir.path().join("b"), "").expect("entry should be written");
        fs::write(dir.path().join("a"), "").expect("entry should be written");

        assert_eq!(
            (
                dir_entry_names(dir.path()).map_err(|error| error.kind()),
                dir_entry_names(&dir.path().join("absent")).map_err(|error| error.kind()),
            ),
            (Ok(vec!["a".to_string(), "b".to_string()]), Ok(Vec::new())),
        );
    }
}
