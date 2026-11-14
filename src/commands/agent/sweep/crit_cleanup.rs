use std::collections::HashSet;
use std::fs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;

use super::super::crit;
use super::super::store;

pub(super) fn clear_stale_links(sessions_dir: &Path, dry_run: bool) -> Result<usize> {
    let live_crit_ports = crit_sessions_dir().and_then(|crit_sessions_dir| {
        read_live_crit_ports(&crit_sessions_dir, crate::infra::process::is_process_alive)
    });
    let live_crit_ports = match live_crit_ports {
        Ok(ports) => Some(ports),
        Err(error) => {
            tracing::warn!(
                event = "agent.sweep.crit_daemon_sessions_read_failed",
                error = %error,
            );
            None
        }
    };
    clear_stale_links_with_port_probe_and_live_crit_ports(
        sessions_dir,
        dry_run,
        is_crit_port_live,
        live_crit_ports.as_ref(),
    )
}

#[cfg(test)]
fn clear_stale_links_with_port_probe(
    sessions_dir: &Path,
    dry_run: bool,
    is_endpoint_live: impl FnMut(SocketAddr) -> bool,
) -> Result<usize> {
    clear_stale_links_with_port_probe_and_live_crit_ports(
        sessions_dir,
        dry_run,
        is_endpoint_live,
        Some(&HashSet::new()),
    )
}

fn clear_stale_links_with_port_probe_and_live_crit_ports(
    sessions_dir: &Path,
    dry_run: bool,
    is_endpoint_live: impl FnMut(SocketAddr) -> bool,
    live_crit_ports: Option<&HashSet<u16>>,
) -> Result<usize> {
    let mut is_endpoint_live = is_endpoint_live;
    let candidates = store::list_all_sessions_from(sessions_dir)?
        .into_iter()
        .filter(|session| {
            session
                .crit_urls
                .iter()
                .chain(session.web_uis.iter().map(|web_ui| &web_ui.url))
                .any(|url| local_crit_address(url).is_some())
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Ok(0);
    }
    let live_ports = candidates
        .iter()
        .flat_map(|session| {
            session
                .crit_urls
                .iter()
                .chain(session.web_uis.iter().map(|web_ui| &web_ui.url))
        })
        .filter_map(|url| local_crit_address(url))
        .collect::<HashSet<_>>()
        .into_iter()
        .filter(|endpoint| is_endpoint_live(*endpoint))
        .collect::<HashSet<_>>();
    let mut cleared = 0;

    for listed_session in candidates {
        let stale_count = listed_session
            .crit_urls
            .iter()
            .filter(|url| is_stale_crit_url(url, &live_ports, live_crit_ports))
            .count()
            + listed_session
                .web_uis
                .iter()
                .filter(|web_ui| is_stale_web_ui(web_ui, &live_ports, live_crit_ports))
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

        let mut removed = 0;
        store::update_session_in(sessions_dir, &listed_session.session_id, |session| {
            let previous_len = session.crit_urls.len();
            session
                .crit_urls
                .retain(|url| !is_stale_crit_url(url, &live_ports, live_crit_ports));
            let previous_web_ui_len = session.web_uis.len();
            session
                .web_uis
                .retain(|web_ui| !is_stale_web_ui(web_ui, &live_ports, live_crit_ports));
            removed = previous_len - session.crit_urls.len() + previous_web_ui_len
                - session.web_uis.len();
            removed > 0
        })?;
        cleared += removed;
    }

    Ok(cleared)
}

fn is_crit_port_live(endpoint: SocketAddr) -> bool {
    TcpStream::connect_timeout(&endpoint, Duration::from_secs(1)).is_ok()
}

fn is_stale_local_crit_url(url: &str, live_endpoints: &HashSet<SocketAddr>) -> bool {
    local_crit_address(url).is_some_and(|endpoint| !live_endpoints.contains(&endpoint))
}

fn is_stale_crit_url(
    url: &str,
    live_endpoints: &HashSet<SocketAddr>,
    live_crit_ports: Option<&HashSet<u16>>,
) -> bool {
    local_crit_address(url).is_some_and(|endpoint| {
        live_crit_ports.is_some_and(|ports| {
            !ports.contains(&endpoint.port()) && !live_endpoints.contains(&endpoint)
        })
    })
}

fn is_stale_web_ui(
    web_ui: &super::super::types::WebUiRegistration,
    live_endpoints: &HashSet<SocketAddr>,
    live_crit_ports: Option<&HashSet<u16>>,
) -> bool {
    if web_ui.name.eq_ignore_ascii_case("crit") {
        is_stale_crit_url(&web_ui.url, live_endpoints, live_crit_ports)
    } else {
        is_stale_local_crit_url(&web_ui.url, live_endpoints)
    }
}

#[derive(Deserialize)]
struct CritDaemonSession {
    pid: u32,
    port: u16,
}

fn crit_sessions_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".crit/sessions"))
        .context("HOME is not set while locating crit daemon sessions")
}

fn read_live_crit_ports(
    sessions_dir: &Path,
    mut is_process_alive: impl FnMut(u32) -> bool,
) -> Result<HashSet<u16>> {
    let entries = match fs::read_dir(sessions_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashSet::new()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read {}", sessions_dir.display()));
        }
    };

    let mut live_ports = HashSet::new();
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
            live_ports.insert(session.port);
        }
    }
    Ok(live_ports)
}

fn local_crit_address(url: &str) -> Option<SocketAddr> {
    let port = crit::crit_port_from_url(url).ok()?;
    let host = reqwest::Url::parse(url).ok()?.host_str()?.to_string();
    if host == "localhost" || host.ends_with(".localhost") {
        return Some(SocketAddr::from((Ipv4Addr::LOCALHOST, port)));
    }

    match host.parse::<IpAddr>().ok()? {
        IpAddr::V4(address) if address.is_loopback() => {
            Some(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
        }
        IpAddr::V6(address) if address.is_loopback() => {
            Some(SocketAddr::from((Ipv6Addr::LOCALHOST, port)))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::{BTreeMap, BTreeSet, HashSet};
    use std::fs;
    use std::net::TcpListener;
    use std::path::PathBuf;

    use chrono::Utc;
    use tempfile::TempDir;

    use super::super::super::store;
    use super::super::super::types::{Session, SessionStatus, WebUiRegistration};
    use super::*;

    fn session(crit_urls: &[&str]) -> Session {
        let timestamp = Utc::now();
        Session {
            archived_at: None,
            pr_context: None,
            session_id: "fixture-session".to_string(),
            crit_urls: crit_urls.iter().map(|url| (*url).to_string()).collect(),
            web_uis: Vec::new(),
            pending_human_review_ids: BTreeSet::new(),
            cwd: PathBuf::from("/tmp/fixture-worktree"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            created_at: timestamp,
            updated_at: timestamp,
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

    fn write_crit_daemon_session(dir: &Path, name: &str, pid: u32, port: u16) {
        fs::write(
            dir.join(name),
            serde_json::json!({ "pid": pid, "port": port }).to_string(),
        )
        .expect("crit daemon session should be written");
    }

    fn load_session(sessions_dir: &Path, session_id: &str) -> Session {
        store::load_session_from(sessions_dir, session_id)
            .expect("session should load")
            .expect("session should exist")
    }

    #[test]
    fn clears_only_unreachable_local_crit_links_and_preserves_live_and_remote_urls() {
        let before = session(&[
            "http://localhost:41001/review",
            "http://127.0.0.1:41002/review",
            "http://[::1]:41003/review",
            "http://localhost:41004/review",
            "http://review.localhost:41006/review",
            "https://review.example.test:41005/review",
            "http://localhost/review",
            "not a URL",
        ]);
        let dir = sessions_dir(&before);
        let live_endpoints = HashSet::from([
            SocketAddr::from((Ipv6Addr::LOCALHOST, 41003)),
            SocketAddr::from((Ipv4Addr::LOCALHOST, 41004)),
        ]);
        let actual = clear_stale_links_with_port_probe(dir.path(), false, |endpoint| {
            live_endpoints.contains(&endpoint)
        })
        .expect("crit cleanup pass should succeed");

        assert_eq!(
            (
                actual,
                load_session(dir.path(), &before.session_id).crit_urls
            ),
            (
                3,
                vec![
                    "http://[::1]:41003/review".to_string(),
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
        let actual = clear_stale_links_with_port_probe(dir.path(), true, |_| false)
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
    fn clears_stale_web_ui_links_and_preserves_live_ones() {
        let mut before = session(&[]);
        before.web_uis = vec![
            WebUiRegistration {
                name: "Editor".to_owned(),
                url: "http://localhost:42501/design/sample/".to_owned(),
                exposed_url: None,
            },
            WebUiRegistration {
                name: "Preview".to_owned(),
                url: "http://127.0.0.1:42502/preview/".to_owned(),
                exposed_url: None,
            },
        ];
        let dir = sessions_dir(&before);

        let actual = clear_stale_links_with_port_probe(dir.path(), false, |endpoint| {
            endpoint.port() == 42502
        })
        .expect("web UI cleanup pass should succeed");

        assert_eq!(
            (actual, load_session(dir.path(), &before.session_id).web_uis,),
            (
                1,
                vec![WebUiRegistration {
                    name: "Preview".to_owned(),
                    url: "http://127.0.0.1:42502/preview/".to_owned(),
                    exposed_url: None,
                }],
            ),
        );
    }

    #[test]
    fn stale_web_ui_cleanup_only_removes_the_registration() {
        let mut before = session(&[]);
        before.web_uis = vec![WebUiRegistration {
            name: "Editor".to_owned(),
            url: "http://localhost:42501/design/sample/".to_owned(),
            exposed_url: Some("https://public.example.test/design/sample/".to_owned()),
        }];
        let dir = sessions_dir(&before);
        let result = clear_stale_links_with_port_probe(dir.path(), false, |_| false)
            .expect("stale web UI cleanup should succeed");
        let stored = load_session(dir.path(), &before.session_id);

        assert_eq!((result, stored.web_uis), (1, Vec::new()));
    }

    #[test]
    fn keeps_crit_links_for_live_daemons_even_when_tcp_probe_fails() {
        let before = session(&[
            "http://localhost:44001/review",
            "http://localhost:44002/review",
        ]);
        let mut before = before;
        before.web_uis = vec![
            WebUiRegistration {
                name: "crit".to_owned(),
                url: "http://localhost:44003/review".to_owned(),
                exposed_url: None,
            },
            WebUiRegistration {
                name: "Editor".to_owned(),
                url: "http://localhost:44004/design/sample/".to_owned(),
                exposed_url: None,
            },
        ];
        let dir = sessions_dir(&before);
        let crit_sessions_dir = TempDir::new().expect("crit session directory should be created");
        write_crit_daemon_session(crit_sessions_dir.path(), "live.json", 51001, 44001);
        write_crit_daemon_session(crit_sessions_dir.path(), "dead.json", 51002, 44002);
        write_crit_daemon_session(crit_sessions_dir.path(), "live-web-ui.json", 51001, 44003);
        let live_crit_ports = read_live_crit_ports(crit_sessions_dir.path(), |pid| pid == 51001)
            .expect("crit daemon sessions should be read");

        let actual = clear_stale_links_with_port_probe_and_live_crit_ports(
            dir.path(),
            false,
            |_| false,
            Some(&live_crit_ports),
        )
        .expect("crit cleanup pass should succeed");

        let after = load_session(dir.path(), &before.session_id);
        assert_eq!(
            (actual, after.crit_urls, after.web_uis),
            (
                2,
                vec!["http://localhost:44001/review".to_owned()],
                vec![WebUiRegistration {
                    name: "crit".to_owned(),
                    url: "http://localhost:44003/review".to_owned(),
                    exposed_url: None,
                }],
            ),
        );
    }

    #[test]
    fn keeps_crit_links_when_daemon_records_cannot_be_read_and_cleans_other_web_uis() {
        let mut before = session(&["http://localhost:44501/review"]);
        before.web_uis = vec![WebUiRegistration {
            name: "Editor".to_owned(),
            url: "http://localhost:44502/design/sample/".to_owned(),
            exposed_url: None,
        }];
        let dir = sessions_dir(&before);

        let actual = clear_stale_links_with_port_probe_and_live_crit_ports(
            dir.path(),
            false,
            |_| false,
            None,
        )
        .expect("crit cleanup pass should succeed");

        let after = load_session(dir.path(), &before.session_id);
        assert_eq!(
            (actual, after.crit_urls, after.web_uis),
            (
                1,
                vec!["http://localhost:44501/review".to_owned()],
                Vec::new(),
            ),
        );
    }

    #[test]
    fn probes_each_local_crit_port_once() {
        let before = session(&[
            "http://localhost:43001/review",
            "http://127.0.0.1:43001/review",
            "http://localhost:43002/review",
        ]);
        let dir = sessions_dir(&before);
        let mut probed_ports = Cell::new(Vec::new());
        let result = clear_stale_links_with_port_probe(dir.path(), false, |endpoint| {
            probed_ports.get_mut().push(endpoint);
            endpoint.port() == 43002
        });
        let mut probed_ports = probed_ports.into_inner();
        probed_ports.sort_unstable();

        assert_eq!(
            (
                result.map_err(|error| format!("{error:#}")),
                probed_ports,
                load_session(dir.path(), &before.session_id).crit_urls,
            ),
            (
                Ok(2),
                vec![
                    SocketAddr::from((Ipv4Addr::LOCALHOST, 43001)),
                    SocketAddr::from((Ipv4Addr::LOCALHOST, 43002)),
                ],
                vec!["http://localhost:43002/review".to_string()],
            ),
        );
    }

    #[test]
    fn keeps_a_link_when_its_loopback_port_accepts_tcp_connections() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .expect("test listener should bind to an ephemeral loopback port");
        let endpoint = listener.local_addr().expect("listener has a local address");
        let live_url = format!("http://127.0.0.1:{}/review", endpoint.port());
        let before = session(&[&live_url]);
        let dir = sessions_dir(&before);

        let actual = clear_stale_links_with_port_probe(dir.path(), false, is_crit_port_live)
            .expect("crit cleanup pass should succeed while the port accepts connections");

        assert_eq!(
            (
                actual,
                load_session(dir.path(), &before.session_id).crit_urls
            ),
            (0, vec![live_url]),
        );
    }
}
