use std::path::Path;

use anyhow::{Context, Result};

use super::wait_state::reconcile_registered_session;
use super::{claim_marker, clients, marker_path, remove_marker_if_equals};
use crate::commands::agent::types::Session;
use crate::infra::process;

pub(in crate::commands::agent::crit) fn ensure_started(
    port: u16,
    session: Option<&Session>,
) -> Result<()> {
    ensure_started_with_status(port, session, None)
}

pub(in crate::commands::agent::crit) fn ensure_started_with_status(
    port: u16,
    session: Option<&Session>,
    status: Option<&serde_json::Value>,
) -> Result<()> {
    let marker = marker_path(port)?;
    let Some(reservation) = claim_marker(&marker, || clients::remove_clients(port))? else {
        if let Some(session) = session
            && let Err(error) = reconcile_registered_session(port, &session.session_id, status)
        {
            tracing::warn!(
                event = "agent.crit.monitor.wait_state_update_failed",
                port,
                error = %error,
            );
        }
        return Ok(());
    };
    let pane_id = session
        .and_then(|session| session.tmux_info.as_ref().map(|info| info.pane_id.as_str()))
        .unwrap_or_default();
    let session_id = session.map(|session| session.session_id.as_str());

    let result = (|| {
        let pid = super::monitor_pid_for_port(port, status, super::status::crit_status)?;
        let executable =
            std::env::current_exe().context("failed to resolve the armyknife executable")?;
        let mut args = vec![
            "agent".to_string(),
            "crit".to_string(),
            "monitor".to_string(),
            format!("--port={port}"),
            format!("--pid={pid}"),
            format!("--reservation={reservation}"),
        ];
        if let Some(session_id) = session_id {
            args.push(format!("--session={session_id}"));
        }
        if !pane_id.is_empty() {
            args.push(format!("--pane={pane_id}"));
        }

        let worker_pid =
            process::spawn_detached_with_pid(&executable, args, Some(Path::new("/")), &[])
                .context("failed to spawn crit lifecycle monitor")?;
        tracing::info!(
            event = "agent.crit.monitor.started",
            port,
            worker_pid,
            session = session_id.unwrap_or("none"),
        );
        Ok(())
    })();

    if result.is_err() {
        remove_marker_if_equals(&marker, &reservation);
    }
    result
}
