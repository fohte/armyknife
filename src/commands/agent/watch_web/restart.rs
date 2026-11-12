use std::ffi::{OsStr, OsString};
use std::fs;
use std::future::Future;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::Duration;

use tokio::sync::watch;
use tokio::time;

use crate::infra::process;

pub(super) const POLL_INTERVAL: Duration = Duration::from_secs(1);
pub(super) const EXEC_RETRY_INTERVAL: Duration = Duration::from_secs(10);
pub(super) const SERVER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const RESTART_MARKER: &str = "ARMYKNIFE_WATCH_WEB_RESTART";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ServerOutcome {
    Exited,
    Restart,
}

pub(super) async fn wait_for_server_or_restart<S, R, E>(
    server: S,
    replacement: R,
    restart_sender: watch::Sender<bool>,
    shutdown_timeout: Duration,
) -> Result<ServerOutcome, E>
where
    S: Future<Output = Result<(), E>>,
    R: Future<Output = ()>,
{
    tokio::pin!(server);

    tokio::select! {
        result = &mut server => {
            result?;
            Ok(ServerOutcome::Exited)
        }
        () = replacement => {
            restart_sender.send_replace(true);
            match time::timeout(shutdown_timeout, &mut server).await {
                Ok(result) => result?,
                Err(_) => tracing::warn!(
                    timeout = ?shutdown_timeout,
                    "watch-web servers did not shut down before the restart deadline"
                ),
            }
            Ok(ServerOutcome::Restart)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ExecutableIdentity {
    device: u64,
    inode: u64,
    mode: u32,
}

impl ExecutableIdentity {
    pub(super) fn read(path: &Path) -> io::Result<Self> {
        let metadata = fs::metadata(path)?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
        })
    }

    fn matches(self, path: &Path) -> io::Result<bool> {
        Ok(self == Self::read(path)?)
    }
}

pub(super) fn is_restarted_process() -> bool {
    std::env::var_os(RESTART_MARKER).as_deref() == Some(OsStr::new("1"))
}

pub(super) async fn wait_for_replacement(
    executable: &Path,
    identity: ExecutableIdentity,
    poll_interval: Duration,
) {
    loop {
        time::sleep(poll_interval).await;
        match identity.matches(executable) {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(
                    executable = %executable.display(),
                    error = %error,
                    "failed to inspect watch-web executable"
                );
            }
        }
    }
}

pub(super) fn exec_replacement_if_changed<E>(
    executable: &Path,
    initial_identity: ExecutableIdentity,
    args: &[OsString],
    exec: E,
) -> io::Result<bool>
where
    E: FnOnce(&Path, &[OsString], &[(&str, &str)]) -> io::Result<()>,
{
    if initial_identity.matches(executable)? {
        return Ok(false);
    }

    exec(executable, args, &[(RESTART_MARKER, "1")])?;
    Ok(true)
}

pub(super) fn exec_replacement(
    executable: &Path,
    initial_identity: ExecutableIdentity,
    args: &[OsString],
) -> io::Result<bool> {
    exec_replacement_if_changed(
        executable,
        initial_identity,
        args,
        |path, args, extra_env| Err(process::exec_replace_with_env(path, args, extra_env)),
    )
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::io;
    use std::time::Duration;

    use rstest::rstest;
    use tokio::sync::watch;

    use super::{ExecutableIdentity, ServerOutcome};

    #[rstest]
    #[case::graceful_shutdown(true)]
    #[case::shutdown_deadline(false)]
    #[tokio::test]
    async fn continues_restarting_after_shutdown_or_deadline(#[case] shutdown_on_signal: bool) {
        let (restart_sender, restart_receiver) = watch::channel(false);
        let mut server_receiver = restart_receiver.clone();
        let server = async move {
            if shutdown_on_signal {
                server_receiver.changed().await.map_err(io::Error::other)?;
                Ok(())
            } else {
                std::future::pending::<io::Result<()>>().await
            }
        };
        let result = super::wait_for_server_or_restart(
            server,
            async {},
            restart_sender,
            if shutdown_on_signal {
                Duration::from_secs(1)
            } else {
                Duration::from_millis(1)
            },
        )
        .await;

        assert_eq!(
            (
                result.expect("shutdown deadline should permit restart"),
                *restart_receiver.borrow()
            ),
            (ServerOutcome::Restart, true),
        );
    }

    #[test]
    fn passes_the_replacement_path_arguments_and_marker_to_exec() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let executable = directory.path().join("a");
        let replacement = directory.path().join("a.new");
        fs::write(&executable, "old binary").expect("initial executable should be written");
        fs::write(&replacement, "new binary").expect("replacement should be written");
        let initial_identity = ExecutableIdentity::read(&executable)
            .expect("initial executable identity should be readable");

        fs::rename(&replacement, &executable).expect("replacement should be atomic");

        let args = [OsString::from("agent"), OsString::from("watch-web")];
        let mut observed = None;
        let result = super::exec_replacement_if_changed(
            &executable,
            initial_identity,
            &args,
            |path, args, extra_env| {
                observed = Some((
                    path.to_path_buf(),
                    args.to_vec(),
                    extra_env
                        .iter()
                        .map(|(key, value)| (key.to_string(), value.to_string()))
                        .collect::<Vec<_>>(),
                ));
                Err(io::Error::other("exec boundary is captured in this test"))
            },
        );

        assert_eq!(
            (result.is_err(), observed),
            (
                true,
                Some((
                    executable,
                    args.to_vec(),
                    vec![("ARMYKNIFE_WATCH_WEB_RESTART".to_owned(), "1".to_owned(),),],
                )),
            ),
        );
    }
}
