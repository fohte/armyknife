use std::cell::Cell;
use std::collections::HashSet;
use std::net::IpAddr;
use std::path::Path;

use anyhow::Result;

use super::super::crit;
use super::super::store;

pub(super) fn clear_stale_links(sessions_dir: &Path, dry_run: bool) -> Result<usize> {
    clear_stale_links_with_status(sessions_dir, dry_run, crit::live_crit_ports)
}

fn clear_stale_links_with_status(
    sessions_dir: &Path,
    dry_run: bool,
    mut read_live_ports: impl FnMut() -> Result<HashSet<u16>>,
) -> Result<usize> {
    let live_ports = read_live_ports()?;
    let mut cleared = 0;

    for listed_session in store::list_all_sessions_from(sessions_dir)? {
        let stale_count = listed_session
            .crit_urls
            .iter()
            .filter(|url| is_stale_local_crit_url(url, &live_ports))
            .count();
        if stale_count == 0 {
            continue;
        }

        tracing::info!(
            event = "agent.sweep.stale_crit_links_found",
            session = %listed_session.session_id,
            count = stale_count,
            dry_run,
        );
        if dry_run {
            continue;
        }

        let removed = Cell::new(0);
        store::update_session_in(sessions_dir, &listed_session.session_id, |session| {
            let previous_len = session.crit_urls.len();
            session
                .crit_urls
                .retain(|url| !is_stale_local_crit_url(url, &live_ports));
            removed.set(previous_len - session.crit_urls.len());
            removed.get() > 0
        })?;
        cleared += removed.get();
    }

    Ok(cleared)
}

fn is_stale_local_crit_url(url: &str, live_ports: &HashSet<u16>) -> bool {
    let Ok(port) = crit::crit_port_from_url(url) else {
        return false;
    };
    if live_ports.contains(&port) {
        return false;
    }

    reqwest::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .is_some_and(|host| {
            host == "localhost"
                || host.ends_with(".localhost")
                || host
                    .strip_prefix('[')
                    .and_then(|host| host.strip_suffix(']'))
                    .unwrap_or(&host)
                    .parse::<IpAddr>()
                    .is_ok_and(|address| address.is_loopback())
        })
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet, HashSet};
    use std::path::PathBuf;

    use chrono::Utc;
    use tempfile::TempDir;

    use super::super::super::store;
    use super::super::super::types::{Session, SessionStatus};
    use super::*;

    fn session(crit_urls: &[&str]) -> Session {
        let timestamp = Utc::now();
        Session {
            archived_at: None,
            pr_context: None,
            session_id: "fixture-session".to_string(),
            crit_urls: crit_urls.iter().map(|url| (*url).to_string()).collect(),
            pending_human_review_ids: BTreeSet::new(),
            cwd: PathBuf::from("/tmp/fixture-worktree"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            created_at: timestamp,
            updated_at: timestamp,
            last_message: None,
            current_tool: None,
            label: None,
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            child_launch: Default::default(),
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: BTreeSet::new(),
            pending_agent_task_ids: BTreeSet::new(),
            pending_permission_agent_ids: BTreeSet::new(),
            pending_permission_request_ids: BTreeMap::new(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            agent_status: None,
            agent_status_clear_pending: false,
            engine: Default::default(),
            model: None,
            effort: None,
        }
    }

    fn sessions_dir(session: &Session) -> TempDir {
        let dir = TempDir::new().expect("temporary directory should be created");
        store::save_session_to(dir.path(), session).expect("session should be saved");
        dir
    }

    fn load_session(sessions_dir: &Path, session_id: &str) -> Session {
        store::load_session_from(sessions_dir, session_id)
            .expect("session should load")
            .expect("session should exist")
    }

    #[test]
    fn clears_only_stopped_local_crit_links_and_preserves_live_and_remote_urls() {
        let before = session(&[
            "http://localhost:41001/review",
            "http://127.0.0.1:41002/review",
            "http://[::1]:41003/review",
            "http://localhost:41004/review",
            "https://review.example.test:41005/review",
            "http://localhost/review",
            "not a URL",
        ]);
        let dir = sessions_dir(&before);
        let actual =
            clear_stale_links_with_status(dir.path(), false, || Ok(HashSet::from([41004])))
                .expect("crit cleanup pass should succeed");

        assert_eq!(
            (
                actual,
                load_session(dir.path(), &before.session_id).crit_urls
            ),
            (
                3,
                vec![
                    "http://localhost:41004/review".to_string(),
                    "https://review.example.test:41005/review".to_string(),
                    "http://localhost/review".to_string(),
                    "not a URL".to_string(),
                ],
            ),
        );
    }

    #[test]
    fn dry_run_keeps_stale_links_unchanged() {
        let before = session(&["http://localhost:42001/review"]);
        let dir = sessions_dir(&before);
        let actual = clear_stale_links_with_status(dir.path(), true, || Ok(HashSet::new()))
            .expect("crit cleanup pass should succeed");

        assert_eq!(
            (
                actual,
                load_session(dir.path(), &before.session_id).crit_urls,
            ),
            (0, before.crit_urls),
        );
    }

    #[test]
    fn crit_status_failure_keeps_all_links_unchanged() {
        let before = session(&["http://localhost:43001/review"]);
        let dir = sessions_dir(&before);
        let status_reads = Cell::new(0);
        let result = clear_stale_links_with_status(dir.path(), false, || {
            status_reads.set(status_reads.get() + 1);
            anyhow::bail!("fixture crit status failure")
        });

        assert_eq!(
            (
                result.map_err(|error| format!("{error:#}")),
                status_reads.get(),
                load_session(dir.path(), &before.session_id).crit_urls,
            ),
            (
                Err("fixture crit status failure".to_string()),
                1,
                before.crit_urls,
            ),
        );
    }
}
