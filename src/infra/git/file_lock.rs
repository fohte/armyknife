use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;

use anyhow::Context;

use super::error::Result;

pub(super) fn open_lock_file(path: &Path, label: &str) -> Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .with_context(|| format!("Failed to open {label} lock at {}", path.display()))
}

pub(super) fn try_lock_exclusive(file: &File) -> io::Result<bool> {
    // SAFETY: `file` remains open for the duration of the flock call.
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        return Ok(true);
    }

    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::WouldBlock {
        Ok(false)
    } else {
        Err(error)
    }
}

pub(super) fn lock_exclusive(file: &File, label: &str) -> Result<()> {
    // SAFETY: `file` remains open for the duration of the flock call.
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    if result != 0 {
        return Err(anyhow::anyhow!(
            "Failed to acquire {label} lock: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(())
}

pub(super) fn unlock(file: &File) {
    // SAFETY: `file` remains open for the duration of the flock call.
    unsafe {
        libc::flock(file.as_raw_fd(), libc::LOCK_UN);
    }
}
