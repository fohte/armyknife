use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::LockGuard;

pub(crate) const FINISH_ACK_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewFinishReason {
    /// A review was finished by a generic external request.
    ExternalRequest,
    /// A PR reply push finished the review.
    RepliesPushed,
}

impl ReviewFinishReason {
    pub(crate) const fn signal_byte(self) -> u8 {
        match self {
            Self::ExternalRequest => b'0',
            Self::RepliesPushed => b'p',
        }
    }

    pub(crate) const fn from_signal_byte(byte: u8) -> Self {
        match byte {
            b'p' => Self::RepliesPushed,
            _ => Self::ExternalRequest,
        }
    }
}

pub(crate) fn finish_request_fifo_path(document_path: &Path) -> PathBuf {
    path_with_suffix(document_path, ".finish")
}

pub(crate) fn finish_ack_fifo_path(document_path: &Path) -> PathBuf {
    path_with_suffix(document_path, ".finish.ack")
}

pub(super) fn path_with_suffix(document_path: &Path, suffix: &str) -> PathBuf {
    let mut path = document_path.as_os_str().to_os_string();
    path.push(suffix);
    path.into()
}

pub(super) fn create_fifo_at_path(fifo_path: &Path) -> super::Result<PathBuf> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    match fs::symlink_metadata(fifo_path) {
        Ok(metadata) if metadata.file_type().is_fifo() => fs::remove_file(fifo_path)?,
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "refusing to replace a non-FIFO review signal path",
            )
            .into());
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let c_path = CString::new(fifo_path.as_os_str().as_bytes()).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("FIFO path contains interior NUL byte: {error}"),
        )
    })?;

    // SAFETY: `c_path` is a valid NUL-terminated C string owned by this scope.
    // Mode 0o600 restricts the FIFO to the current user, matching the access
    // the document itself has.
    let result = unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) };
    if result != 0 {
        return Err(io::Error::last_os_error().into());
    }

    Ok(fifo_path.to_path_buf())
}

pub(super) fn remove_fifo_if_present(fifo_path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(fifo_path) {
        Ok(metadata) if metadata.file_type().is_fifo() => fs::remove_file(fifo_path),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "refusing to remove a non-FIFO review signal path",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub(crate) fn open_fifo_reader(fifo_path: &Path) -> super::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(fifo_path)?;
    if !file.metadata()?.file_type().is_fifo() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "review signal path is not a FIFO",
        )
        .into());
    }
    Ok(file)
}

pub(super) fn try_read_fifo_signal_byte(file: &mut File) -> super::Result<Option<u8>> {
    let mut byte = [0_u8; 1];
    match file.read(&mut byte) {
        Ok(bytes_read) => Ok((bytes_read > 0).then_some(byte[0])),
        Err(error)
            if error.kind() == io::ErrorKind::WouldBlock
                || error.kind() == io::ErrorKind::Interrupted =>
        {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

pub(crate) struct ReviewDocumentOperationLock {
    file: File,
}

impl ReviewDocumentOperationLock {
    pub(crate) fn acquire(document_path: &Path) -> io::Result<Self> {
        let lock_path = LockGuard::lock_path(document_path);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&lock_path)?;
        if !file.metadata()?.file_type().is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "review lock path is not a regular file",
            ));
        }

        // SAFETY: `file` remains open while this guard owns the exclusive flock.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(io::Error::last_os_error());
        }

        if !LockGuard::is_locked_with_exclusive_file_lock(document_path, &file) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "active review lock changed before it could be acquired",
            ));
        }

        Ok(Self { file })
    }

    pub(crate) fn is_active(&self, document_path: &Path) -> bool {
        LockGuard::is_locked_with_exclusive_file_lock(document_path, &self.file)
    }
}

impl Drop for ReviewDocumentOperationLock {
    fn drop(&mut self) {
        // SAFETY: `file` remains open for the lifetime of this guard.
        let _ = unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
    }
}

pub(crate) fn wait_for_finish_ack(reader: &mut File, timeout: Duration) -> io::Result<u8> {
    let deadline = Instant::now() + timeout;
    loop {
        let mut byte = [0_u8; 1];
        match reader.read(&mut byte) {
            Ok(bytes_read) if bytes_read > 0 => return Ok(byte[0]),
            Ok(_) => {}
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock
                    || error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }

        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "review process did not acknowledge the finish request",
            ));
        }
        std::thread::sleep(super::FIFO_POLL_INTERVAL);
    }
}

/// Ask an active review to finish and wait for its completion handler.
///
/// Returns `false` when no review currently owns the document.
pub(crate) fn request_review_finish(
    document_path: &Path,
    reason: ReviewFinishReason,
) -> super::Result<bool> {
    let mut document_paths = vec![document_path.to_path_buf()];
    if let Ok(canonical_path) = fs::canonicalize(document_path)
        && canonical_path != document_path
    {
        document_paths.push(canonical_path);
    }

    for document_path in document_paths {
        if request_review_finish_at_path(&document_path, reason)? {
            return Ok(true);
        }
    }

    Ok(false)
}

fn request_review_finish_at_path(
    document_path: &Path,
    reason: ReviewFinishReason,
) -> super::Result<bool> {
    let operation_lock = match ReviewDocumentOperationLock::acquire(document_path) {
        Ok(operation_lock) => operation_lock,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };

    if !operation_lock.is_active(document_path) {
        return Ok(false);
    }

    let mut ack_reader = open_fifo_reader(&finish_ack_fifo_path(document_path))?;
    match signal_review_finish_and_wait_with_reason(
        operation_lock,
        &finish_request_fifo_path(document_path),
        &mut ack_reader,
        reason,
    )? {
        b'0' => Ok(true),
        b'1' => {
            Err(io::Error::other("review completion handler rejected the finish request").into())
        }
        acknowledgement => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unexpected review finish acknowledgement: {acknowledgement}"),
        )
        .into()),
    }
}

pub(crate) fn signal_review_finish_and_wait(
    operation_lock: ReviewDocumentOperationLock,
    finish_fifo_path: &Path,
    ack_reader: &mut File,
) -> io::Result<u8> {
    signal_review_finish_and_wait_with_reason(
        operation_lock,
        finish_fifo_path,
        ack_reader,
        ReviewFinishReason::ExternalRequest,
    )
}

fn signal_review_finish_and_wait_with_reason(
    operation_lock: ReviewDocumentOperationLock,
    finish_fifo_path: &Path,
    ack_reader: &mut File,
    reason: ReviewFinishReason,
) -> io::Result<u8> {
    super::try_signal_fifo_with_byte(finish_fifo_path, reason.signal_byte())?;
    drop(operation_lock);
    wait_for_finish_ack(ack_reader, FINISH_ACK_TIMEOUT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;

    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    struct ActiveReview {
        _temp_dir: TempDir,
        document_path: PathBuf,
        _review_lock: super::super::LockGuard,
    }

    #[fixture]
    fn review_document() -> (TempDir, PathBuf) {
        let temp_dir = TempDir::new().expect("tempdir");
        let document_path = temp_dir.path().join("review.md");
        fs::write(&document_path, "review").expect("write document");
        (temp_dir, document_path)
    }

    #[fixture]
    fn active_review(review_document: (TempDir, PathBuf)) -> ActiveReview {
        let (temp_dir, document_path) = review_document;
        let review_lock = super::super::LockGuard::acquire(&document_path).expect("review lock");
        create_fifo_at_path(&finish_request_fifo_path(&document_path)).expect("create finish fifo");
        create_fifo_at_path(&finish_ack_fifo_path(&document_path))
            .expect("create acknowledgement fifo");
        ActiveReview {
            _temp_dir: temp_dir,
            document_path,
            _review_lock: review_lock,
        }
    }

    #[rstest]
    fn request_review_finish_returns_false_when_no_review_is_active(
        review_document: (TempDir, PathBuf),
    ) {
        let (_temp_dir, document_path) = review_document;
        let expected = false;

        assert_eq!(
            request_review_finish(&document_path, ReviewFinishReason::ExternalRequest)
                .expect("request finish"),
            expected
        );
    }

    #[rstest]
    #[case::accepted(b'0', Ok(true))]
    #[case::rejected(b'1', Err(()))]
    fn request_review_finish_signals_active_review_and_waits_for_acknowledgement(
        active_review: ActiveReview,
        #[case] acknowledgement: u8,
        #[case] expected: std::result::Result<bool, ()>,
    ) {
        let document_path = active_review.document_path;
        let finish_fifo_path = finish_request_fifo_path(&document_path);
        let ack_fifo_path = finish_ack_fifo_path(&document_path);
        let symlink_path = document_path.with_file_name("review-link.md");
        std::os::unix::fs::symlink(&document_path, &symlink_path).expect("create document link");

        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let review_actor = thread::spawn(move || -> super::super::Result<()> {
            let mut finish_reader = open_fifo_reader(&finish_fifo_path)?;
            ready_sender
                .send(())
                .map_err(|error| io::Error::other(error.to_string()))?;
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if try_read_fifo_signal_byte(&mut finish_reader)?.is_some() {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "finish request was not received",
                    )
                    .into());
                }
                thread::sleep(Duration::from_millis(10));
            }
            super::super::try_signal_fifo_with_byte(&ack_fifo_path, acknowledgement)?;
            Ok(())
        });
        ready_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("review actor ready");

        let outcome = (
            request_review_finish(&symlink_path, ReviewFinishReason::ExternalRequest)
                .map_err(|_| ()),
            review_actor
                .join()
                .expect("review actor thread")
                .map_err(|error| format!("{error:#}")),
        );
        assert_eq!(outcome, (expected, Ok(())));
    }

    #[rstest]
    fn request_review_finish_after_push_identifies_the_completion_reason(
        active_review: ActiveReview,
    ) {
        let document_path = active_review.document_path;
        let finish_fifo_path = finish_request_fifo_path(&document_path);
        let ack_fifo_path = finish_ack_fifo_path(&document_path);
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let review_actor = thread::spawn(move || -> super::super::Result<u8> {
            let mut finish_reader = open_fifo_reader(&finish_fifo_path)?;
            ready_sender
                .send(())
                .map_err(|error| io::Error::other(error.to_string()))?;
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if let Some(signal_byte) = try_read_fifo_signal_byte(&mut finish_reader)? {
                    super::super::try_signal_fifo_with_byte(&ack_fifo_path, b'0')?;
                    return Ok(signal_byte);
                }
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "finish request was not received",
                    )
                    .into());
                }
                thread::sleep(Duration::from_millis(10));
            }
        });
        ready_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("review actor ready");

        let outcome = (
            request_review_finish(&document_path, ReviewFinishReason::RepliesPushed)
                .map_err(|_| ()),
            review_actor
                .join()
                .expect("review actor thread")
                .map_err(|_| ()),
        );
        assert_eq!(outcome, (Ok(true), Ok(b'p')));
    }
}
