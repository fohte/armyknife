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

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FdWake {
    Exited,
    Readable,
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

/// Blocks until one of `pids` exits or `fd` becomes readable.
///
/// macOS uses kqueue. Other platforms and kqueue setup failures poll the process and descriptor
/// at the fallback interval. `fd` must remain open for the duration of this call. Some readiness
/// backends can report a FIFO's EOF as readable; callers must handle EOF without re-registering in
/// a loop. The polling fallback sleeps after non-readable events, and `kill(pid, 0)` considers an
/// unreaped child alive.
pub(crate) fn wait_for_exit_or_readable_fd(
    pids: &[u32],
    fd: std::os::fd::RawFd,
    timeout: Option<Duration>,
) -> FdWake {
    #[cfg(target_os = "macos")]
    {
        if let Ok(wake) = wait_for_exit_or_readable_fd_with_kqueue(pids, fd, timeout) {
            return wake;
        }
    }
    poll_for_exit_or_readable_fd(pids, fd, timeout)
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

fn poll_for_exit_or_readable_fd(
    pids: &[u32],
    fd: std::os::fd::RawFd,
    timeout: Option<Duration>,
) -> FdWake {
    let deadline = timeout.map(|timeout| Instant::now() + timeout);
    loop {
        let exited = pids
            .iter()
            .copied()
            .filter(|&pid| !is_process_alive(pid))
            .collect::<Vec<_>>();
        if !exited.is_empty() {
            return FdWake::Exited;
        }

        let wait = match deadline {
            Some(deadline) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return FdWake::TimedOut;
                }
                remaining.min(FALLBACK_POLL_INTERVAL)
            }
            None => FALLBACK_POLL_INTERVAL,
        };
        let timeout_ms = i32::try_from(wait.as_millis()).unwrap_or(i32::MAX).max(1);
        let mut descriptor = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `descriptor` is a valid one-element poll array for the open fd.
        let result = unsafe { libc::poll(&mut descriptor, 1, timeout_ms) };
        if result > 0 {
            if descriptor.revents & libc::POLLIN != 0 {
                return FdWake::Readable;
            }
            thread::sleep(wait);
        }
        if result == -1 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            thread::sleep(wait);
        }
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
fn register_process_exit_watches(
    kq: std::os::fd::RawFd,
    pids: &[u32],
) -> io::Result<Option<Vec<u32>>> {
    let mut exited = Vec::new();
    for &pid in pids {
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
        match register_kevent(kq, &change) {
            Ok(()) => {}
            Err(error) if error.raw_os_error() == Some(libc::ESRCH) => exited.push(pid),
            Err(error) => return Err(error),
        }
    }
    Ok((!exited.is_empty()).then_some(exited))
}

#[cfg(target_os = "macos")]
fn wait_for_kqueue_events(
    kq: std::os::fd::RawFd,
    capacity: usize,
    timeout: Option<Duration>,
) -> io::Result<Option<Vec<libc::kevent>>> {
    use std::ptr;

    let mut events = vec![new_kevent(0, 0, 0, 0); capacity];
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
                kq,
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
            0 => return Ok(None),
            ready => {
                events.truncate(ready as usize);
                return Ok(Some(events));
            }
        }
    }
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

    if let Some(exited) = register_process_exit_watches(kq.as_raw_fd(), pids)? {
        return Ok(Wake::Exited(exited));
    }
    if dir_changed(dir, known_entries) {
        return Ok(Wake::DirChanged);
    }

    let Some(events) = wait_for_kqueue_events(kq.as_raw_fd(), pids.len() + 1, timeout)? else {
        return Ok(Wake::TimedOut);
    };
    let exited = events
        .iter()
        .filter(|event| event.filter == libc::EVFILT_PROC)
        .map(|event| event.ident as u32)
        .collect::<Vec<_>>();
    // Only the directory watch is left when no process exited.
    Ok(if exited.is_empty() {
        Wake::DirChanged
    } else {
        Wake::Exited(exited)
    })
}

#[cfg(target_os = "macos")]
fn wait_for_exit_or_readable_fd_with_kqueue(
    pids: &[u32],
    fd: std::os::fd::RawFd,
    timeout: Option<Duration>,
) -> io::Result<FdWake> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    // SAFETY: `kqueue` takes no arguments and returns a new descriptor or -1.
    let raw = unsafe { libc::kqueue() };
    if raw == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `raw` was just created and nothing else owns it; `OwnedFd` closes it.
    let kq = unsafe { OwnedFd::from_raw_fd(raw) };
    register_kevent(
        kq.as_raw_fd(),
        &new_kevent(fd as libc::uintptr_t, libc::EVFILT_READ, libc::EV_ADD, 0),
    )?;

    if register_process_exit_watches(kq.as_raw_fd(), pids)?.is_some() {
        return Ok(FdWake::Exited);
    }

    let Some(events) = wait_for_kqueue_events(kq.as_raw_fd(), pids.len() + 1, timeout)? else {
        return Ok(FdWake::TimedOut);
    };
    Ok(
        if events.iter().any(|event| event.filter == libc::EVFILT_PROC) {
            FdWake::Exited
        } else {
            FdWake::Readable
        },
    )
}

#[cfg(test)]
mod tests {
    use std::ffi::CString;
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::io::Write;
    use std::os::fd::{AsRawFd, RawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::OpenOptionsExt;
    use std::process::{Child, ChildStdin, Command, Stdio};
    use std::thread::JoinHandle;

    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::*;

    type Wait = fn(&[u32], &Path, &[String], Option<Duration>) -> Wake;
    type FdWait = fn(&[u32], RawFd, Option<Duration>) -> FdWake;

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

    #[cfg(target_os = "macos")]
    fn wait_for_exit_or_readable_fd_with_kqueue_only(
        pids: &[u32],
        fd: RawFd,
        timeout: Option<Duration>,
    ) -> FdWake {
        wait_for_exit_or_readable_fd_with_kqueue(pids, fd, timeout)
            .expect("kqueue should be usable in tests")
    }

    #[fixture]
    fn dir() -> TempDir {
        TempDir::new().expect("temporary directory should be created")
    }

    struct ReadableFifo {
        _temp: TempDir,
        reader: File,
        writer: Option<File>,
    }

    fn readable_fifo(with_writer: bool) -> ReadableFifo {
        let temp = TempDir::new().expect("temporary directory should be created");
        let path = temp.path().join("signal");
        let c_path = CString::new(path.as_os_str().as_bytes()).expect("FIFO path should be valid");
        // SAFETY: `c_path` is a valid NUL-terminated path inside the test's temporary directory.
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        let reader = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path)
            .expect("FIFO reader should open");
        let writer = with_writer.then(|| {
            OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&path)
                .expect("FIFO writer should open")
        });
        ReadableFifo {
            _temp: temp,
            reader,
            writer,
        }
    }

    #[rstest]
    #[case::default(wait_for_exit_or_readable_fd as FdWait)]
    #[case::polling(poll_for_exit_or_readable_fd as FdWait)]
    #[cfg_attr(target_os = "macos", case::kqueue(wait_for_exit_or_readable_fd_with_kqueue_only as FdWait))]
    fn descriptor_wait_times_out_for_an_empty_fifo_with_a_writer(#[case] wait: FdWait) {
        let fifo = readable_fifo(true);

        assert_eq!(
            wait(
                &[],
                fifo.reader.as_raw_fd(),
                Some(Duration::from_millis(50))
            ),
            FdWake::TimedOut,
        );
    }

    #[rstest]
    #[case::default(wait_for_exit_or_readable_fd as FdWait)]
    #[case::polling(poll_for_exit_or_readable_fd as FdWait)]
    #[cfg_attr(target_os = "macos", case::kqueue(wait_for_exit_or_readable_fd_with_kqueue_only as FdWait))]
    fn descriptor_wait_reports_fifo_data_as_readable(#[case] wait: FdWait) {
        let mut fifo = readable_fifo(true);
        fifo.writer
            .as_mut()
            .expect("FIFO writer should be open")
            .write_all(b"x")
            .expect("FIFO byte should be written");

        assert_eq!(
            wait(&[], fifo.reader.as_raw_fd(), Some(Duration::from_secs(1))),
            FdWake::Readable,
        );
    }

    #[rstest]
    #[case::default(wait_for_exit_or_readable_fd as FdWait)]
    #[case::polling(poll_for_exit_or_readable_fd as FdWait)]
    #[cfg_attr(target_os = "macos", case::kqueue(wait_for_exit_or_readable_fd_with_kqueue_only as FdWait))]
    fn descriptor_wait_reports_a_gone_pid_as_exited(#[case] wait: FdWait) {
        let fifo = readable_fifo(true);

        assert_eq!(
            wait(
                &[u32::MAX],
                fifo.reader.as_raw_fd(),
                Some(Duration::from_secs(1))
            ),
            FdWake::Exited,
        );
    }

    #[cfg(target_os = "macos")]
    #[rstest]
    fn kqueue_does_not_report_writerless_fifo_eof_as_readable() {
        let fifo = readable_fifo(false);
        let first = wait_for_exit_or_readable_fd_with_kqueue_only(
            &[],
            fifo.reader.as_raw_fd(),
            Some(Duration::from_millis(50)),
        );
        let second = wait_for_exit_or_readable_fd_with_kqueue_only(
            &[],
            fifo.reader.as_raw_fd(),
            Some(Duration::from_millis(50)),
        );

        assert_eq!((first, second), (FdWake::TimedOut, FdWake::TimedOut));
    }

    fn thread_cpu_time() -> Duration {
        let mut time = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: `time` points to a valid writable timespec.
        assert_eq!(
            unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) },
            0
        );
        Duration::new(
            u64::try_from(time.tv_sec).expect("thread cpu time should be non-negative"),
            u32::try_from(time.tv_nsec).expect("nanoseconds should fit in u32"),
        )
    }

    #[test]
    fn polling_descriptor_wait_sleeps_on_fifo_hangup() {
        let fifo = readable_fifo(false);
        let cpu_time_before = thread_cpu_time();

        assert_eq!(
            poll_for_exit_or_readable_fd(
                &[],
                fifo.reader.as_raw_fd(),
                Some(Duration::from_millis(200)),
            ),
            FdWake::TimedOut,
        );
        assert!(thread_cpu_time() - cpu_time_before < Duration::from_millis(100));
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
    #[case::sorted_entries(&["b", "a"], "", Ok(vec!["a".to_string(), "b".to_string()]))]
    #[case::missing_directory_has_none(&[], "absent", Ok(Vec::new()))]
    fn lists_entry_names(
        dir: TempDir,
        #[case] files: &[&str],
        #[case] subdir: &str,
        #[case] expected: Result<Vec<String>, io::ErrorKind>,
    ) {
        for file in files {
            fs::write(dir.path().join(file), "").expect("entry should be written");
        }

        assert_eq!(
            dir_entry_names(&dir.path().join(subdir)).map_err(|error| error.kind()),
            expected,
        );
    }
}
