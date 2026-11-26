use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Result, bail};

use super::super::signal::{LibcSignalSender, SignalSender};
use super::super::store;
use super::super::types::Session;
use super::monitor::{crit_status, pid_for_port};
use super::url::parse_port;

pub(super) fn stop_expired_session_daemons(session: &Session, sessions_dir: &Path) -> Result<()> {
    let remaining_sessions = store::all_session_rows_from(sessions_dir)?;
    let ports = unshared_crit_ports(session, &remaining_sessions);
    if ports.is_empty() {
        return Ok(());
    }

    let status = crit_status()?;
    stop_daemons_for_ports(&ports, &status, &LibcSignalSender)
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
    use std::cell::RefCell;
    use std::io;

    use rstest::rstest;

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

    #[rstest]
    fn sends_sigterm_only_to_unshared_crit_daemons() {
        let expired = make_session(
            "expired-session",
            &[
                "http://review.example.test:43120/review",
                "http://review.example.test:43121/review",
                "http://review.example.test:43120/another-path",
            ],
        );
        let remaining = [make_session(
            "remaining-session",
            &["http://review.example.test:43120/review"],
        )];
        let ports = unshared_crit_ports(&expired, &remaining);
        let status = serde_json::json!({
            "sessions": [
                {"port": 43120, "pid": 9120},
                {"port": 43121, "pid": 9121},
            ]
        });
        let sender = RecordingSender::default();

        stop_daemons_for_ports(&ports, &status, &sender)
            .expect("unshared daemon should stop successfully");

        assert_eq!(
            (ports, sender.calls.into_inner()),
            (BTreeSet::from([43121]), vec![(9121, libc::SIGTERM)],),
        );
    }
}
