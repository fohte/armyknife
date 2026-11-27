//! Pid files of crit clients that announced themselves with `crit client-started`.
//!
//! The daemon does not track its clients, so the caller's process lifetime stands in for the
//! client's: a live pid file means a review is being waited on.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::{lock_marker, marker_is_active, marker_path};
use crate::commands::agent::store;
use crate::infra::process;

fn clients_dir(port: u16) -> Result<PathBuf> {
    Ok(store::crit_dir()?.join(format!("{port}.clients")))
}

/// Records a client only while a monitor owns `port`, so no pid file outlives its monitor.
///
/// The marker lock is the one the monitor holds while it removes the clients and its marker on
/// exit, so a client is either removed with the monitor or refused here.
pub(super) fn record_client_if_monitored(port: u16, pid: u32) -> Result<()> {
    record_if_monitored(&marker_path(port)?, || {
        record_client_in(&clients_dir(port)?, pid)
    })
}

fn record_if_monitored(marker: &Path, record: impl FnOnce() -> Result<()>) -> Result<()> {
    let _lock = lock_marker(marker)?;
    if !marker_is_active(marker)? {
        bail!("the crit monitor stopped before the client could be recorded");
    }
    record()
}

/// Removes pid files of exited processes while looking for a live one.
pub(super) fn has_live_client(port: u16) -> Result<bool> {
    let (_, live) = scan_in(&clients_dir(port)?, process::is_process_alive)?;
    Ok(!live.is_empty())
}

pub(super) fn remove_clients(port: u16) {
    if let Ok(dir) = clients_dir(port) {
        let _ = fs::remove_dir_all(dir);
    }
}

/// What the monitor blocks on between events: the daemon, the live announced clients, and the
/// clients directory itself.
///
/// `client-started` writes the session wait state itself, so the monitor has to look again when
/// a client is announced, not only when one exits.
pub(super) struct ClientsWatch {
    dir: PathBuf,
    names: Vec<String>,
    live: Vec<u32>,
}

impl ClientsWatch {
    pub(super) fn new(port: u16) -> Result<Self> {
        Ok(Self {
            dir: clients_dir(port)?,
            names: Vec::new(),
            live: Vec::new(),
        })
    }

    /// Looks at the clients directory again and reports whether a live client remains.
    ///
    /// `exited` are pids whose exit event already arrived: `kill(pid, 0)` still reports such a
    /// process alive until its parent reaps it, and no further event would correct that.
    pub(super) fn scan(&mut self, exited: &[u32]) -> Result<bool> {
        // A kqueue directory watch needs the directory to exist, and it is absent until the first
        // client is announced and again after a stale one is discarded.
        let scanned = fs::create_dir_all(&self.dir)
            .with_context(|| {
                format!(
                    "failed to create crit clients directory {}",
                    self.dir.display()
                )
            })
            .and_then(|()| {
                scan_in(&self.dir, |pid| {
                    !exited.contains(&pid) && process::is_process_alive(pid)
                })
            });
        match scanned {
            Ok((names, live)) => {
                self.names = names;
                self.live = live;
                Ok(!self.live.is_empty())
            }
            Err(error) => {
                self.names.clear();
                self.live.clear();
                Err(error)
            }
        }
    }

    /// Blocks until the daemon or a live client exits or the directory's entries change.
    pub(super) fn wait(&self, daemon_pid: u32) -> process::Wake {
        let pids = std::iter::once(daemon_pid)
            .chain(self.live.iter().copied())
            .collect::<Vec<_>>();
        process::wait_for_exit_or_dir_change(&pids, &self.dir, &self.names, None)
    }
}

fn record_client_in(dir: &Path, pid: u32) -> Result<()> {
    fs::create_dir_all(dir)
        .with_context(|| format!("failed to create crit clients directory {}", dir.display()))?;
    let path = dir.join(pid.to_string());
    fs::write(&path, "").with_context(|| format!("failed to record crit client {}", path.display()))
}

/// Prunes the pid files of exited processes and returns what is left: the sorted entry names
/// (pid files and anything else) and the pids that are alive.
///
/// The names are derived from one listing instead of read again after pruning, so an entry added
/// in between differs from them when they are compared with the directory later.
fn scan_in(dir: &Path, is_process_alive: impl Fn(u32) -> bool) -> Result<(Vec<String>, Vec<u32>)> {
    let mut names = process::dir_entry_names(dir)
        .with_context(|| format!("failed to read crit clients directory {}", dir.display()))?;
    let mut live = Vec::new();
    names.retain(|name| {
        let Ok(pid) = name.parse::<u32>() else {
            return true;
        };
        if is_process_alive(pid) {
            live.push(pid);
            return true;
        }
        // An entry that stays on disk must stay listed: dropping it would make every later wait
        // see a change and return at once.
        match fs::remove_file(dir.join(name)) {
            Ok(()) => false,
            Err(error) => error.kind() != std::io::ErrorKind::NotFound,
        }
    });
    Ok((names, live))
}

#[cfg(test)]
mod tests {
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::*;

    #[fixture]
    fn dir() -> TempDir {
        TempDir::new().expect("temporary directory should be created")
    }

    fn file_names(dir: &Path) -> Vec<String> {
        let mut names = fs::read_dir(dir)
            .expect("clients directory should be readable")
            .map(|entry| {
                entry
                    .expect("directory entry should be readable")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    #[rstest]
    #[case::live_client_survives_pruning(&["101", "202"], &["101"], &[101])]
    #[case::only_exited_clients(&["202", "303"], &[], &[])]
    #[case::non_pid_files_are_left_alone(&["101", "notes"], &["101", "notes"], &[101])]
    #[case::no_clients(&[], &[], &[])]
    fn prunes_exited_clients_and_reports_what_is_left(
        dir: TempDir,
        #[case] files: &[&str],
        #[case] expected_names: &[&str],
        #[case] expected_live: &[u32],
    ) -> Result<()> {
        for file in files {
            fs::write(dir.path().join(file), "")?;
        }

        let (names, live) = scan_in(dir.path(), |pid| pid == 101)?;

        assert_eq!(
            (names, live, file_names(dir.path())),
            (
                expected_names.iter().map(|name| name.to_string()).collect(),
                expected_live.to_vec(),
                expected_names.iter().map(|name| name.to_string()).collect(),
            ),
        );
        Ok(())
    }

    #[rstest]
    fn an_entry_that_cannot_be_removed_stays_listed(dir: TempDir) -> Result<()> {
        // A directory named like a pid cannot be removed with `remove_file`.
        fs::create_dir(dir.path().join("303"))?;

        let (names, live) = scan_in(dir.path(), |_| false)?;

        assert_eq!(
            (names, live, file_names(dir.path())),
            (vec!["303".to_string()], Vec::new(), vec!["303".to_string()]),
        );
        Ok(())
    }

    #[rstest]
    fn scan_treats_a_pid_with_an_exit_event_as_gone_even_if_it_still_answers(
        dir: TempDir,
    ) -> Result<()> {
        // The test runner's own pid stands for an exited process that is not yet reaped.
        let unreaped = std::process::id();
        let mut watch = ClientsWatch {
            dir: dir.path().join("43210.clients"),
            names: Vec::new(),
            live: Vec::new(),
        };
        record_client_in(&watch.dir, unreaped)?;

        let before_event = watch.scan(&[])?;
        let after_event = watch.scan(&[unreaped])?;

        assert_eq!(
            (
                before_event,
                after_event,
                watch.names,
                watch.live,
                file_names(&watch.dir)
            ),
            (true, false, Vec::new(), Vec::new(), Vec::new()),
        );
        Ok(())
    }

    #[rstest]
    fn scan_creates_the_directory_the_watch_needs(dir: TempDir) -> Result<()> {
        let mut watch = ClientsWatch {
            dir: dir.path().join("43210.clients"),
            names: Vec::new(),
            live: Vec::new(),
        };

        let live = watch.scan(&[])?;

        assert_eq!((live, watch.dir.is_dir()), (false, true));
        Ok(())
    }

    #[rstest]
    fn wait_covers_the_daemon_and_every_live_client(dir: TempDir) {
        // Pids that cannot be processes report as exited at once, in the order they were watched.
        let watch = ClientsWatch {
            dir: dir.path().to_path_buf(),
            names: Vec::new(),
            live: vec![u32::MAX],
        };

        assert_eq!(watch.wait(0), process::Wake::Exited(vec![0, u32::MAX]),);
    }

    #[rstest]
    #[case::monitor_owns_the_port(Some(std::process::id().to_string()), Ok(()), 1)]
    #[case::no_marker(
        None,
        Err("the crit monitor stopped before the client could be recorded"),
        0
    )]
    #[case::monitor_exited(
        Some("0".to_string()),
        Err("the crit monitor stopped before the client could be recorded"),
        0
    )]
    fn clients_are_recorded_only_while_a_monitor_owns_the_port(
        dir: TempDir,
        #[case] marker_contents: Option<String>,
        #[case] expected: std::result::Result<(), &str>,
        #[case] expected_records: usize,
    ) -> Result<()> {
        let marker = dir.path().join("43210.monitor");
        if let Some(contents) = marker_contents {
            fs::write(&marker, contents)?;
        }
        let records = std::cell::Cell::new(0);

        let result = record_if_monitored(&marker, || {
            records.set(records.get() + 1);
            Ok(())
        })
        .map_err(|error| error.to_string());

        assert_eq!(
            (result, records.get()),
            (expected.map_err(str::to_string), expected_records),
        );
        Ok(())
    }

    #[rstest]
    fn missing_directory_has_no_live_client(dir: TempDir) -> Result<()> {
        assert_eq!(
            scan_in(&dir.path().join("absent"), |_| true)?,
            (Vec::new(), Vec::new()),
        );
        Ok(())
    }

    #[rstest]
    fn recording_the_same_client_twice_keeps_one_file(dir: TempDir) -> Result<()> {
        let clients = dir.path().join("43210.clients");

        record_client_in(&clients, 4242)?;
        record_client_in(&clients, 4242)?;

        assert_eq!(file_names(&clients), vec!["4242".to_string()]);
        Ok(())
    }
}
