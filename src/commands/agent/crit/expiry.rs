use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use super::super::signal::{LibcSignalSender, SignalSender};
use super::super::store;
use super::super::types::Session;
use super::monitor::{crit_status, pid_for_port};
use super::url::parse_port;

pub(super) fn stop_expired_session_daemons(session: &Session, sessions_dir: &Path) -> Result<()> {
    stop_expired_session_daemons_with(
        session,
        sessions_dir,
        store::all_session_rows_from,
        crit_status,
        &LibcSignalSender,
    )
}

fn stop_expired_session_daemons_with(
    session: &Session,
    sessions_dir: &Path,
    load_remaining_sessions: impl FnOnce(&Path) -> Result<Vec<Session>>,
    read_status: impl FnOnce() -> Result<serde_json::Value>,
    sender: &impl SignalSender,
) -> Result<()> {
    let candidate_ports = session
        .crit_urls
        .iter()
        .filter_map(|url| parse_port(url).ok())
        .collect::<BTreeSet<_>>();
    if candidate_ports.is_empty() {
        return Ok(());
    }

    let _port_locks = candidate_ports
        .iter()
        .map(|port| lock_port(sessions_dir, *port))
        .collect::<Result<Vec<_>>>()?;
    let remaining_sessions = load_remaining_sessions(sessions_dir)?;
    let ports = unshared_crit_ports(session, &remaining_sessions);
    if ports.is_empty() {
        return Ok(());
    }

    let status = read_status()?;
    stop_daemons_for_ports(&ports, &status, sender)
}

pub(super) fn lock_port(sessions_dir: &Path, port: u16) -> Result<File> {
    // Keep lock inodes stable so concurrent registrations cannot bypass an active lock.
    let lock_dir = crit_port_lock_dir(sessions_dir);
    fs::create_dir_all(&lock_dir)?;
    let lock_path = lock_dir.join(format!("{port}.lock"));
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    file.lock()?;
    Ok(file)
}

fn crit_port_lock_dir(sessions_dir: &Path) -> PathBuf {
    sessions_dir.join("crit-port-locks")
}

fn unshared_crit_ports(session: &Session, remaining_sessions: &[Session]) -> BTreeSet<u16> {
    let linked_elsewhere = remaining_sessions
        .iter()
        .flat_map(|session| session.crit_urls.iter())
        .filter_map(|url| parse_port(url).ok())
        .collect::<BTreeSet<_>>();

    session
        .crit_urls
        .iter()
        .filter_map(|url| parse_port(url).ok())
        .filter(|port| !linked_elsewhere.contains(port))
        .collect()
}

fn stop_daemons_for_ports(
    ports: &BTreeSet<u16>,
    status: &serde_json::Value,
    sender: &impl SignalSender,
) -> Result<()> {
    let mut failures = Vec::new();
    for port in ports {
        let Some(pid) = pid_for_port(status, *port) else {
            continue;
        };
        if let Err(error) = sender.send(pid, libc::SIGTERM) {
            failures.push(format!("port {port} (pid {pid}): {error}"));
        }
    }

    if !failures.is_empty() {
        bail!(
            "failed to stop expired session crit daemons: {}",
            failures.join("; ")
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::cell::RefCell;
    use std::io;

    use super::*;
    use crate::commands::agent::types::{Engine, SessionStatus};

    #[derive(Default)]
    struct RecordingSender {
        calls: RefCell<Vec<(u32, i32)>>,
    }

    impl SignalSender for RecordingSender {
        fn send(&self, pid: u32, signal: i32) -> io::Result<()> {
            self.calls.borrow_mut().push((pid, signal));
            Ok(())
        }
    }

    fn make_session(session_id: &str, crit_urls: &[&str]) -> Session {
        Session {
            launch_id: None,
            archived_at: None,
            pr_context: None,
            created_pr_urls: Vec::new(),
            session_id: session_id.to_string(),
            crit_urls: crit_urls.iter().map(|url| (*url).to_string()).collect(),
            crit_not_waiting_urls: Default::default(),
            web_uis: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: Path::new("/tmp/example-worktree").to_path_buf(),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Ended,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            last_prompted_at: None,
            last_message: None,
            current_tool: None,
            current_tool_name: None,
            label: None,
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            child_launch: Default::default(),
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: Default::default(),
            pending_agent_task_ids: Default::default(),
            pending_permission_agent_ids: Default::default(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            agent_status: None,
            agent_status_clear_pending: false,
            engine: Engine::Claude,
            model: None,
            effort: None,
        }
    }

    #[test]
    fn sends_sigterm_only_to_unshared_crit_daemons() {
        let temp_dir = tempfile::TempDir::new().expect("temporary directory should be created");
        let expired = make_session(
            "expired-session",
            &[
                "http://review.example.test:43120/review",
                "http://review.example.test:43121/review",
                "http://review.example.test:43120/another-path",
            ],
        );
        let mut remaining = make_session(
            "remaining-session",
            &["http://review.example.test:43120/review"],
        );
        remaining.archived_at = Some(chrono::Utc::now());
        store::save_session_to(temp_dir.path(), &remaining)
            .expect("remaining session should be saved");
        let status = serde_json::json!({
            "sessions": [
                {"port": 43120, "pid": 9120},
                {"port": 43121, "pid": 9121},
            ]
        });
        let status_reads = Cell::new(0);
        let sender = RecordingSender::default();

        stop_expired_session_daemons_with(
            &expired,
            temp_dir.path(),
            store::all_session_rows_from,
            || {
                status_reads.set(status_reads.get() + 1);
                Ok(status.clone())
            },
            &sender,
        )
        .expect("unshared daemon should stop successfully");

        assert_eq!(
            (sender.calls.into_inner(), status_reads.get()),
            (vec![(9121, libc::SIGTERM)], 1),
        );
    }
}
