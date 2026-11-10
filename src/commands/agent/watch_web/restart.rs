use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::time;

use crate::infra::process;

const POLL_INTERVAL: Duration = Duration::from_secs(1);
const RESTART_MARKER: &str = "ARMYKNIFE_WATCH_WEB_RESTART";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ExecutableIdentity {
    device: u64,
    inode: u64,
}

impl ExecutableIdentity {
    pub(super) fn read(path: &Path) -> io::Result<Self> {
        let metadata = fs::metadata(path)?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }

    fn matches(self, path: &Path) -> io::Result<bool> {
        Ok(self == Self::read(path)?)
    }
}

pub(super) fn is_restarted_process() -> bool {
    std::env::var_os(RESTART_MARKER).as_deref() == Some(OsStr::new("1"))
}

pub(super) async fn restart_when_replaced(
    executable: std::path::PathBuf,
    initial_identity: ExecutableIdentity,
    args: Vec<OsString>,
) -> Result<()> {
    let mut interval = time::interval(POLL_INTERVAL);
    loop {
        interval.tick().await;
        match initial_identity.matches(&executable) {
            Ok(true) => {}
            Ok(false) => {
                let error =
                    process::exec_replace_with_env(&executable, args, &[(RESTART_MARKER, "1")]);
                return Err(error).context("failed to restart watch-web after binary replacement");
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to inspect executable {}", executable.display())
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::ExecutableIdentity;

    #[test]
    fn detects_an_atomic_executable_replacement() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let executable = directory.path().join("a");
        let replacement = directory.path().join("a.new");
        fs::write(&executable, "old binary").expect("initial executable should be written");
        fs::write(&replacement, "new binary").expect("replacement should be written");
        let initial_identity = ExecutableIdentity::read(&executable)
            .expect("initial executable identity should be readable");

        fs::rename(&replacement, &executable).expect("replacement should be atomic");

        assert_ne!(
            initial_identity,
            ExecutableIdentity::read(&executable)
                .expect("replacement executable identity should be readable"),
        );
    }
}
