use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Deserialize)]
struct CritDaemonSession {
    pid: u32,
    port: u16,
    #[serde(default)]
    cwd: Option<String>,
}

pub(in crate::commands::agent) fn crit_sessions_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".crit/sessions"))
        .context("HOME is not set while locating crit daemon sessions")
}

pub(in crate::commands::agent) fn read_live_crit_ports(
    sessions_dir: &Path,
    is_process_alive: impl FnMut(u32) -> bool,
) -> Result<HashSet<u16>> {
    Ok(read_live_crit_sessions(sessions_dir, is_process_alive)?
        .into_iter()
        .map(|session| session.port)
        .collect())
}

pub(super) fn live_pid_for_port_and_cwd(port: u16, cwd: &Path) -> Result<Option<u32>> {
    live_pid_for_port_and_cwd_from(
        &crit_sessions_dir()?,
        port,
        cwd,
        crate::infra::process::is_process_alive,
    )
}

fn live_pid_for_port_and_cwd_from(
    sessions_dir: &Path,
    port: u16,
    cwd: &Path,
    is_process_alive: impl FnMut(u32) -> bool,
) -> Result<Option<u32>> {
    let cwd = normalize_cwd(&cwd.to_string_lossy());
    let mut matching_pids = BTreeSet::new();
    for session in read_live_crit_sessions(sessions_dir, is_process_alive)? {
        if session.port == port
            && session
                .cwd
                .as_deref()
                .is_some_and(|session_cwd| normalize_cwd(session_cwd) == cwd)
        {
            matching_pids.insert(session.pid);
        }
    }

    if matching_pids.len() > 1 {
        bail!("multiple live crit daemons match port {port} and cwd {cwd}");
    }
    Ok(matching_pids.into_iter().next())
}

fn read_live_crit_sessions(
    sessions_dir: &Path,
    mut is_process_alive: impl FnMut(u32) -> bool,
) -> Result<Vec<CritDaemonSession>> {
    let entries = match fs::read_dir(sessions_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read {}", sessions_dir.display()));
        }
    };

    let mut live_sessions = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("failed to read {}", sessions_dir.display()))?;
        if entry
            .path()
            .extension()
            .is_none_or(|extension| extension != "json")
        {
            continue;
        }
        let path = entry.path();
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("failed to read crit daemon session {}", path.display()))?;
        let session: CritDaemonSession = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse crit daemon session {}", path.display()))?;
        if is_process_alive(session.pid) {
            live_sessions.push(session);
        }
    }
    Ok(live_sessions)
}

#[cfg(windows)]
fn normalize_cwd(cwd: &str) -> String {
    cwd.replace('\\', "/")
}

#[cfg(not(windows))]
fn normalize_cwd(cwd: &str) -> String {
    cwd.to_owned()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::live_pid_for_port_and_cwd_from;

    #[fixture]
    fn sessions_dir() -> TempDir {
        tempfile::tempdir().expect("temporary directory should be created")
    }

    fn write_session(dir: &Path, pid: u32, port: u16, cwd: &str) {
        fs::write(
            dir.join("daemon.json"),
            serde_json::json!({ "pid": pid, "port": port, "cwd": cwd }).to_string(),
        )
        .expect("crit daemon registration should be written");
    }

    #[rstest]
    #[case::matching_port_and_cwd(
        43120,
        "/repo/worktree",
        43120,
        "/repo/worktree",
        true,
        Some(9121)
    )]
    #[case::different_port(43121, "/repo/worktree", 43120, "/repo/worktree", true, None)]
    #[case::different_cwd(43120, "/repo/other-worktree", 43120, "/repo/worktree", true, None)]
    #[case::dead_daemon(43120, "/repo/worktree", 43120, "/repo/worktree", false, None)]
    fn resolves_only_a_live_daemon_registered_for_the_port_and_cwd(
        sessions_dir: TempDir,
        #[case] registered_port: u16,
        #[case] registered_cwd: &str,
        #[case] requested_port: u16,
        #[case] requested_cwd: &str,
        #[case] is_alive: bool,
        #[case] expected_pid: Option<u32>,
    ) {
        write_session(sessions_dir.path(), 9121, registered_port, registered_cwd);

        let actual = live_pid_for_port_and_cwd_from(
            sessions_dir.path(),
            requested_port,
            Path::new(requested_cwd),
            |_| is_alive,
        )
        .expect("crit daemon registrations should be read");

        assert_eq!(actual, expected_pid);
    }

    #[rstest]
    #[case::empty_registry(false)]
    #[case::no_matching_port(true)]
    fn returns_no_pid_when_the_registry_has_no_matching_session(
        sessions_dir: TempDir,
        #[case] write_other_port: bool,
    ) {
        if write_other_port {
            write_session(sessions_dir.path(), 9121, 43121, "/repo/worktree");
        }

        let actual = live_pid_for_port_and_cwd_from(
            sessions_dir.path(),
            43120,
            Path::new("/repo/worktree"),
            |_| true,
        )
        .expect("an absent registration should be treated as no daemon");

        assert_eq!(actual, None);
    }
}
