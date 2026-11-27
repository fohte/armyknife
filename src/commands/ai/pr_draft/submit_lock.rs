use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

pub(super) struct SubmitGuard {
    _file: File,
}

impl SubmitGuard {
    fn acquire(draft_path: &Path) -> io::Result<Self> {
        let mut lock_path = draft_path.as_os_str().to_os_string();
        lock_path.push(".submit.lock");
        let lock_path = PathBuf::from(lock_path);
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true).mode(0o600);
        options.custom_flags(libc::O_NOFOLLOW);
        let file = options.open(lock_path)?;

        loop {
            // Keep the lock file in place so concurrent submitters always lock the same inode.
            // SAFETY: `file` stays open for the lifetime of the guard.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
                return Ok(Self { _file: file });
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }
}

pub(super) async fn acquire_submit_lock(draft_path: &Path) -> anyhow::Result<SubmitGuard> {
    let draft_path = draft_path.to_owned();
    Ok(tokio::task::spawn_blocking(move || SubmitGuard::acquire(&draft_path)).await??)
}
