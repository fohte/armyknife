//! Pid files of crit clients that announced themselves with `crit client-started`.
//!
//! The daemon does not track its clients, so the caller's process lifetime stands in for the
//! client's: a live pid file means a review is being waited on.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::commands::agent::store;
use crate::infra::process;

fn clients_dir(port: u16) -> Result<PathBuf> {
    Ok(store::crit_dir()?.join(format!("{port}.clients")))
}

pub(super) fn record_client(port: u16, pid: u32) -> Result<()> {
    record_client_in(&clients_dir(port)?, pid)
}

/// Removes pid files of exited processes while looking for a live one.
pub(super) fn has_live_client(port: u16) -> Result<bool> {
    has_live_client_in(&clients_dir(port)?, process::is_process_alive)
}

pub(super) fn remove_clients(port: u16) {
    if let Ok(dir) = clients_dir(port) {
        let _ = fs::remove_dir_all(dir);
    }
}

fn record_client_in(dir: &Path, pid: u32) -> Result<()> {
    fs::create_dir_all(dir)
        .with_context(|| format!("failed to create crit clients directory {}", dir.display()))?;
    let path = dir.join(pid.to_string());
    fs::write(&path, "").with_context(|| format!("failed to record crit client {}", path.display()))
}

fn has_live_client_in(dir: &Path, is_process_alive: impl Fn(u32) -> bool) -> Result<bool> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| {
                format!("failed to read crit clients directory {}", dir.display())
            });
        }
    };

    let mut live = false;
    for entry in entries {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        if is_process_alive(pid) {
            live = true;
        } else {
            let _ = fs::remove_file(entry.path());
        }
    }
    Ok(live)
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
    #[case::live_client_survives_pruning(&["101", "202"], true, &["101"])]
    #[case::only_exited_clients(&["202", "303"], false, &[])]
    #[case::non_pid_files_are_left_alone(&["101", "notes"], true, &["101", "notes"])]
    #[case::no_clients(&[], false, &[])]
    fn prunes_exited_clients_and_reports_live_ones(
        dir: TempDir,
        #[case] files: &[&str],
        #[case] expected_live: bool,
        #[case] expected_files: &[&str],
    ) -> Result<()> {
        for file in files {
            fs::write(dir.path().join(file), "")?;
        }

        let live = has_live_client_in(dir.path(), |pid| pid == 101)?;

        assert_eq!(
            (live, file_names(dir.path())),
            (
                expected_live,
                expected_files.iter().map(|name| name.to_string()).collect()
            ),
        );
        Ok(())
    }

    #[rstest]
    fn missing_directory_has_no_live_client(dir: TempDir) -> Result<()> {
        assert!(!has_live_client_in(&dir.path().join("absent"), |_| true)?);
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
