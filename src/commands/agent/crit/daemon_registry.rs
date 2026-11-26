use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::Deserialize;

#[derive(Debug, PartialEq, Eq, Deserialize)]
struct CritDaemonSession {
    pid: u32,
    port: u16,
    #[serde(default)]
    started_at: Option<String>,
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

pub(super) fn live_pid_for_port_and_updated_at(
    port: u16,
    updated_at: DateTime<Utc>,
) -> Result<Option<u32>> {
    live_pid_for_port_and_updated_at_from(
        &crit_sessions_dir()?,
        port,
        updated_at,
        crate::infra::process::is_process_alive,
    )
}

fn live_pid_for_port_and_updated_at_from(
    sessions_dir: &Path,
    port: u16,
    updated_at: DateTime<Utc>,
    is_process_alive: impl FnMut(u32) -> bool,
) -> Result<Option<u32>> {
    let mut matching_pids = BTreeSet::new();
    for session in read_live_crit_sessions(sessions_dir, is_process_alive)? {
        if session.port != port {
            continue;
        }
        let started_at = session.started_at.as_deref().with_context(|| {
            format!("crit daemon registration for port {port} has no started_at")
        })?;
        let started_at = DateTime::parse_from_rfc3339(started_at)
            .with_context(|| format!("failed to parse started_at for crit daemon on port {port}"))?
            .with_timezone(&Utc);
        if started_at <= updated_at {
            matching_pids.insert(session.pid);
        }
    }

    if matching_pids.len() > 1 {
        bail!("multiple live crit daemon registrations match port {port}");
    }
    Ok(matching_pids.into_iter().next())
}

fn read_live_crit_sessions(
    sessions_dir: &Path,
    is_process_alive: impl FnMut(u32) -> bool,
) -> Result<Vec<CritDaemonSession>> {
    read_live_crit_sessions_with(sessions_dir, is_process_alive, |path| {
        fs::read_to_string(path)
    })
}

fn read_live_crit_sessions_with(
    sessions_dir: &Path,
    mut is_process_alive: impl FnMut(u32) -> bool,
    mut read_session: impl FnMut(&Path) -> std::io::Result<String>,
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
        let contents = match read_session(&path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to read crit daemon session {}", path.display())
                });
            }
        };
        let session: CritDaemonSession = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse crit daemon session {}", path.display()))?;
        if is_process_alive(session.pid) {
            live_sessions.push(session);
        }
    }
    Ok(live_sessions)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use chrono::{DateTime, Utc};
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::{
        live_pid_for_port_and_updated_at_from, read_live_crit_sessions,
        read_live_crit_sessions_with,
    };

    #[fixture]
    fn sessions_dir() -> TempDir {
        tempfile::tempdir().expect("temporary directory should be created")
    }

    fn write_session(dir: &Path, name: &str, pid: u32, port: u16, started_at: Option<&str>) {
        let session = match started_at {
            Some(started_at) => {
                serde_json::json!({ "pid": pid, "port": port, "started_at": started_at })
            }
            None => serde_json::json!({ "pid": pid, "port": port }),
        };
        fs::write(dir.join(name), session.to_string())
            .expect("crit daemon registration should be written");
    }

    fn session_updated_at() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-01-02T00:00:00Z")
            .expect("session update time should parse")
            .with_timezone(&Utc)
    }

    #[rstest]
    #[case::started_before_session_update("2026-01-01T23:59:59Z", 43120, true, Some(9121))]
    #[case::started_at_session_update("2026-01-02T00:00:00Z", 43120, true, Some(9121))]
    #[case::different_port("2026-01-01T23:59:59Z", 43121, true, None)]
    #[case::started_after_session_update("2026-01-02T00:00:01Z", 43120, true, None)]
    #[case::dead_daemon("2026-01-01T23:59:59Z", 43120, false, None)]
    fn resolves_only_a_live_daemon_started_before_or_at_session_update(
        sessions_dir: TempDir,
        #[case] started_at: &str,
        #[case] requested_port: u16,
        #[case] is_alive: bool,
        #[case] expected_pid: Option<u32>,
    ) {
        write_session(
            sessions_dir.path(),
            "daemon.json",
            9121,
            43120,
            Some(started_at),
        );

        let actual = live_pid_for_port_and_updated_at_from(
            sessions_dir.path(),
            requested_port,
            session_updated_at(),
            |_| is_alive,
        )
        .expect("crit daemon registrations should be read");

        assert_eq!(actual, expected_pid);
    }

    #[rstest]
    fn returns_no_pid_when_registry_has_no_entries(sessions_dir: TempDir) {
        let actual = live_pid_for_port_and_updated_at_from(
            sessions_dir.path(),
            43120,
            session_updated_at(),
            |_| true,
        )
        .expect("an empty registry should return no daemon");

        assert_eq!(actual, None);
    }

    #[rstest]
    #[case::missing(None, "crit daemon registration for port 43120 has no started_at")]
    #[case::invalid(
        Some("invalid"),
        "failed to parse started_at for crit daemon on port 43120"
    )]
    fn returns_error_for_missing_or_invalid_started_at(
        sessions_dir: TempDir,
        #[case] started_at: Option<&str>,
        #[case] expected_error: &str,
    ) {
        write_session(sessions_dir.path(), "daemon.json", 9121, 43120, started_at);

        let actual = live_pid_for_port_and_updated_at_from(
            sessions_dir.path(),
            43120,
            session_updated_at(),
            |_| true,
        )
        .map_err(|error| error.to_string());

        assert_eq!(actual, Err(expected_error.to_string()));
    }

    #[rstest]
    fn rejects_multiple_live_daemons_registered_for_the_same_port(sessions_dir: TempDir) {
        write_session(
            sessions_dir.path(),
            "first.json",
            9121,
            43120,
            Some("2026-01-01T23:59:59Z"),
        );
        write_session(
            sessions_dir.path(),
            "second.json",
            9122,
            43120,
            Some("2026-01-01T23:59:59Z"),
        );

        let actual = live_pid_for_port_and_updated_at_from(
            sessions_dir.path(),
            43120,
            session_updated_at(),
            |_| true,
        )
        .map_err(|error| error.to_string());

        assert_eq!(
            actual,
            Err("multiple live crit daemon registrations match port 43120".to_string()),
        );
    }

    #[rstest]
    fn skips_session_file_removed_before_it_can_be_read(sessions_dir: TempDir) {
        fs::write(sessions_dir.path().join("daemon.json"), "{}")
            .expect("crit daemon registration should be created");

        let actual = read_live_crit_sessions_with(
            sessions_dir.path(),
            |_| true,
            |_| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "registration disappeared",
                ))
            },
        )
        .map_err(|error| error.to_string());

        assert_eq!(actual, Ok(Vec::new()));
    }

    #[rstest]
    fn returns_error_for_unparseable_registry_file(sessions_dir: TempDir) {
        let path = sessions_dir.path().join("daemon.json");
        fs::write(&path, "not-json").expect("crit daemon registration should be created");

        let actual = read_live_crit_sessions(sessions_dir.path(), |_| true)
            .map(|sessions| sessions.len())
            .map_err(|error| error.to_string());

        assert_eq!(
            actual,
            Err(format!(
                "failed to parse crit daemon session {}",
                path.display()
            )),
        );
    }
}
