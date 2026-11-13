use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::LockGuard;

pub(crate) const FINISH_ACK_TIMEOUT: Duration = Duration::from_secs(5);

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

pub(super) fn try_read_fifo_signal(file: &mut File) -> super::Result<bool> {
    let mut byte = [0_u8; 1];
    match file.read(&mut byte) {
        Ok(bytes_read) => Ok(bytes_read > 0),
        Err(error)
            if error.kind() == io::ErrorKind::WouldBlock
                || error.kind() == io::ErrorKind::Interrupted =>
        {
            Ok(false)
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
