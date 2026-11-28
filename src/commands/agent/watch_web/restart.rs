use std::ffi::{OsStr, OsString};
use std::fs;
use std::future::Future;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc::{self, UnboundedReceiver};
use tokio::sync::watch;
use tokio::time;

use crate::infra::process;

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

pub(super) struct ExecutableWatcher {
    _watcher: RecommendedWatcher,
    pub(super) events: UnboundedReceiver<notify::Result<notify::Event>>,
}

impl ExecutableWatcher {
    pub(super) fn new(executable: &Path) -> anyhow::Result<Self> {
        let (sender, events) = mpsc::unbounded_channel::<notify::Result<notify::Event>>();
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = sender.send(event);
        })?;
        register_executable_watches(executable, |path| {
            watcher.watch(path, RecursiveMode::NonRecursive)
        })?;
        Ok(Self {
            _watcher: watcher,
            events,
        })
    }
}

fn executable_watch_directory(executable: &Path) -> anyhow::Result<PathBuf> {
    let executable = fs::canonicalize(executable)?;
    let parent = executable
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| anyhow::anyhow!("executable has no parent directory"))?;
    Ok(parent.to_path_buf())
}

fn register_executable_watches(
    executable: &Path,
    mut register: impl FnMut(&Path) -> notify::Result<()>,
) -> anyhow::Result<()> {
    let launch_directory = executable
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| anyhow::anyhow!("executable has no parent directory"))?;
    let launch_directory = fs::canonicalize(launch_directory)?;
    let target_directory = executable_watch_directory(executable)?;

    register(&launch_directory)?;
    if target_directory != launch_directory {
        register(&target_directory)?;
    }
    Ok(())
}

pub(super) fn create_watcher_or_warn<T, E>(create: impl FnOnce() -> Result<T, E>) -> Option<T>
where
    E: std::fmt::Display,
{
    match create() {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            tracing::warn!(
                %error,
                "failed to watch watch-web executable; automatic restart is disabled"
            );
            None
        }
    }
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
    startup_replacement_pending: &mut bool,
    events: Option<&mut UnboundedReceiver<notify::Result<notify::Event>>>,
) {
    let Some(events) = events else {
        return std::future::pending().await;
    };

    if std::mem::take(startup_replacement_pending) {
        return;
    }

    loop {
        match events.recv().await {
            Some(Ok(event))
                if event.need_rescan()
                    || event
                        .paths
                        .iter()
                        .any(|path| event_path_matches_executable(path, executable)) =>
            {
                if executable_was_replaced(executable, identity) {
                    return;
                }
            }
            Some(Ok(_)) => {}
            Some(Err(error)) => {
                tracing::warn!(%error, "failed to watch watch-web executable");
                if executable_was_replaced(executable, identity) {
                    return;
                }
            }
            None => std::future::pending::<()>().await,
        }
    }
}

fn event_path_matches_executable(event_path: &Path, executable: &Path) -> bool {
    matches!(
        (fs::canonicalize(event_path), fs::canonicalize(executable)),
        (Ok(event_path), Ok(executable)) if event_path == executable
    )
}

fn executable_was_replaced(executable: &Path, identity: ExecutableIdentity) -> bool {
    match identity.matches(executable) {
        Ok(matches) => !matches,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => {
            tracing::warn!(
                executable = %executable.display(),
                error = %error,
                "failed to inspect watch-web executable"
            );
            false
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
    use std::path::PathBuf;
    use std::time::Duration;

    use rstest::{fixture, rstest};
    use tokio::sync::mpsc;
    use tokio::sync::{oneshot, watch};

    use super::{ExecutableIdentity, ServerOutcome};

    struct ExecutableReplacement {
        _directory: tempfile::TempDir,
        executable: PathBuf,
        replacement: PathBuf,
        identity: ExecutableIdentity,
    }

    #[fixture]
    fn executable_replacement() -> ExecutableReplacement {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let executable = directory.path().join("a");
        let replacement = directory.path().join("a.new");
        fs::write(&executable, "old binary").expect("initial executable should be written");
        fs::write(&replacement, "new binary").expect("replacement should be written");
        let identity = ExecutableIdentity::read(&executable)
            .expect("initial executable identity should be readable");
        ExecutableReplacement {
            _directory: directory,
            executable,
            replacement,
            identity,
        }
    }

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

    #[rstest]
    #[tokio::test]
    async fn waits_for_a_matching_event_after_the_executable_identity_changes(
        executable_replacement: ExecutableReplacement,
    ) {
        let ExecutableReplacement {
            _directory,
            executable,
            replacement,
            identity,
        } = executable_replacement;
        let unrelated = executable.with_file_name("unrelated");
        let (sender, mut events) = mpsc::unbounded_channel();
        let mut startup_replacement_pending = false;
        let waiting = super::wait_for_replacement(
            &executable,
            identity,
            &mut startup_replacement_pending,
            Some(&mut events),
        );
        tokio::pin!(waiting);

        sender
            .send(Ok(
                notify::Event::new(notify::EventKind::Any).add_path(executable.clone())
            ))
            .expect("watch event should be delivered");
        let unchanged_identity = tokio::time::timeout(Duration::from_millis(1), &mut waiting)
            .await
            .is_err();

        fs::rename(&replacement, &executable).expect("replacement should be atomic");
        sender
            .send(Ok(
                notify::Event::new(notify::EventKind::Any).add_path(unrelated)
            ))
            .expect("unrelated watch event should be delivered");
        let unrelated_event = tokio::time::timeout(Duration::from_millis(1), &mut waiting)
            .await
            .is_err();

        sender
            .send(Ok(
                notify::Event::new(notify::EventKind::Any).add_path(executable.clone())
            ))
            .expect("matching watch event should be delivered");
        let matching_event = tokio::time::timeout(Duration::from_secs(1), &mut waiting)
            .await
            .is_ok();
        assert_eq!(
            (unchanged_identity, unrelated_event, matching_event),
            (true, true, true),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn rescans_recheck_the_executable_identity(
        executable_replacement: ExecutableReplacement,
    ) {
        let ExecutableReplacement {
            _directory,
            executable,
            replacement,
            identity,
        } = executable_replacement;
        let (sender, mut events) = mpsc::unbounded_channel();
        let mut startup_replacement_pending = false;
        let waiting = super::wait_for_replacement(
            &executable,
            identity,
            &mut startup_replacement_pending,
            Some(&mut events),
        );
        tokio::pin!(waiting);

        fs::rename(&replacement, &executable).expect("replacement should be atomic");
        sender
            .send(Ok(
                notify::Event::new(notify::EventKind::Other).set_flag(notify::event::Flag::Rescan)
            ))
            .expect("rescan event should be delivered");
        assert!(
            tokio::time::timeout(Duration::from_secs(1), &mut waiting)
                .await
                .is_ok()
        );
    }

    #[rstest]
    #[tokio::test]
    async fn restarts_immediately_after_replacement_during_watcher_setup(
        executable_replacement: ExecutableReplacement,
    ) {
        let ExecutableReplacement {
            _directory,
            executable,
            identity,
            ..
        } = executable_replacement;
        let (_sender, mut events) = mpsc::unbounded_channel();
        let mut startup_replacement_pending = true;
        let mut waiting = Box::pin(super::wait_for_replacement(
            &executable,
            identity,
            &mut startup_replacement_pending,
            Some(&mut events),
        ));

        let completed = tokio::time::timeout(Duration::from_secs(1), &mut waiting)
            .await
            .is_ok();
        drop(waiting);
        assert_eq!((completed, startup_replacement_pending), (true, false));
    }

    #[rstest]
    #[tokio::test]
    async fn watcher_errors_recheck_the_executable_identity(
        executable_replacement: ExecutableReplacement,
    ) {
        let ExecutableReplacement {
            _directory,
            executable,
            replacement,
            identity,
        } = executable_replacement;
        let (sender, mut events) = mpsc::unbounded_channel();
        let mut startup_replacement_pending = false;
        let waiting = super::wait_for_replacement(
            &executable,
            identity,
            &mut startup_replacement_pending,
            Some(&mut events),
        );
        tokio::pin!(waiting);

        fs::rename(&replacement, &executable).expect("replacement should be atomic");
        sender
            .send(Err(notify::Error::generic("synthetic watcher error")))
            .expect("watcher error should be delivered");
        let completed = tokio::time::timeout(Duration::from_secs(1), &mut waiting)
            .await
            .is_ok();
        assert!(completed);
    }

    #[rstest]
    #[tokio::test]
    async fn keeps_serving_without_automatic_restart_when_watcher_creation_fails(
        executable_replacement: ExecutableReplacement,
    ) {
        let ExecutableReplacement {
            _directory,
            executable,
            replacement,
            identity,
        } = executable_replacement;
        let mut events = super::create_watcher_or_warn(|| {
            Err::<mpsc::UnboundedReceiver<notify::Result<notify::Event>>, _>(io::Error::other(
                "synthetic watcher error",
            ))
        });
        let watcher_unavailable = events.is_none();
        let (server_exit_sender, server_exit_receiver) = oneshot::channel();
        let server = async move {
            server_exit_receiver.await.map_err(io::Error::other)?;
            Ok::<_, io::Error>(())
        };
        let (restart_sender, restart_receiver) = watch::channel(false);
        let mut startup_replacement_pending = false;
        let replacement_wait = super::wait_for_replacement(
            &executable,
            identity,
            &mut startup_replacement_pending,
            events.as_mut(),
        );
        let waiting = super::wait_for_server_or_restart(
            server,
            replacement_wait,
            restart_sender,
            Duration::from_secs(1),
        );
        tokio::pin!(waiting);

        let server_keeps_running_before_replacement =
            tokio::time::timeout(Duration::from_millis(1), &mut waiting)
                .await
                .is_err();
        fs::rename(&replacement, &executable).expect("replacement should be atomic");
        let server_keeps_running_after_replacement =
            tokio::time::timeout(Duration::from_millis(1), &mut waiting)
                .await
                .is_err();
        server_exit_sender
            .send(())
            .expect("server should stay active until its shutdown signal");
        let outcome = tokio::time::timeout(Duration::from_secs(1), &mut waiting)
            .await
            .expect("server shutdown should complete")
            .expect("server shutdown should succeed");

        assert_eq!(
            (
                watcher_unavailable,
                server_keeps_running_before_replacement,
                server_keeps_running_after_replacement,
                outcome,
                *restart_receiver.borrow()
            ),
            (true, true, true, ServerOutcome::Exited, false),
        );
    }

    #[test]
    fn matches_canonical_event_paths_for_symlinked_executables() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let executable = directory.path().join("a");
        let symlink = directory.path().join("a-link");
        fs::write(&executable, "binary").expect("executable should be written");
        std::os::unix::fs::symlink(&executable, &symlink).expect("symlink should be created");
        let event_path = fs::canonicalize(&executable).expect("event path should be canonical");

        assert!(super::event_path_matches_executable(&event_path, &symlink));
    }

    #[test]
    fn registers_launch_and_target_directories_for_symlinked_executables() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let launch_directory = directory.path().join("bin");
        let target_directory = directory.path().join("libexec");
        fs::create_dir(&launch_directory).expect("launch directory should be created");
        fs::create_dir(&target_directory).expect("target directory should be created");
        let executable = target_directory.join("a");
        let symlink = launch_directory.join("a");
        fs::write(&executable, "binary").expect("executable should be written");
        std::os::unix::fs::symlink(&executable, &symlink).expect("symlink should be created");
        let launch_directory =
            fs::canonicalize(launch_directory).expect("launch directory should be canonical");
        let target_directory =
            fs::canonicalize(target_directory).expect("target directory should be canonical");
        let mut registered_directories = Vec::new();

        super::register_executable_watches(&symlink, |path| {
            registered_directories.push(path.to_path_buf());
            Ok(())
        })
        .expect("watch directories should be registered");

        assert_eq!(
            registered_directories,
            vec![launch_directory, target_directory],
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
