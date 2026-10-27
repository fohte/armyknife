//! Link crit reviews to agent sessions and open them in tmux floating panes.

mod monitor;
mod pane;
mod toggle;
mod url;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use std::time::Duration;

use super::store;
use super::types::{Session, resolve_session_option};
use super::{display_label, repo_name};
use crate::infra::external_tool::ExternalTool;
use crate::infra::notification::{Notification, NotificationAction};
use crate::infra::process;
use crate::infra::tmux;
use crate::shared::caller_pane;
use crate::shared::env_var::EnvVars;
use crate::shared::log::short_run_id;

use self::url::parse_port;

const CRIT_TITLE: &str = "■ crit - Review requested";

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum CritCommands {
    /// Associate a crit review URL with the current agent session.
    Add(AddArgs),

    /// Toggle the latest crit review for a session or pane in a tmux floating pane.
    Open(OpenArgs),

    /// Wait for a crit daemon to stop, then clear its session link.
    #[command(name = "monitor", hide = true)]
    Monitor(monitor::MonitorArgs),
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct AddArgs {
    /// Crit review URL.
    pub url: String,
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct OpenArgs {
    /// Agent session ID to open.
    #[arg(long, conflicts_with = "pane")]
    pub session: Option<String>,

    /// Tmux pane ID whose agent session should be opened.
    #[arg(long, conflicts_with = "session")]
    pub pane: Option<String>,

    /// Wait for the watch popup to close before opening the review.
    #[arg(long, hide = true)]
    pub parent_pid: Option<u32>,
}

pub fn run(command: &CritCommands) -> Result<()> {
    match command {
        CritCommands::Add(args) => add(args),
        CritCommands::Open(args) => open(args),
        CritCommands::Monitor(args) => monitor::run(args),
    }
}

fn add(args: &AddArgs) -> Result<()> {
    let Some(session_id) = EnvVars::load().own_session_id() else {
        if let Some(pane_id) = caller_pane::resolve_caller_pane_id() {
            return add_without_agent_session(args, &pane_id);
        }
        return open_url_in_default_browser(&args.url);
    };

    let port = parse_port(&args.url)?;
    let span = tracing::info_span!(
        "agent.crit.add",
        run_id = %short_run_id(),
        session = %session_id,
        port,
    );
    let _guard = span.enter();
    let result = record_session_url(&session_id, &args.url);
    continue_after_session_recording(&session_id, result, |session| {
        if session.is_some() {
            tracing::info!(event = "agent.crit.add.registered");
        }
        continue_add(args, &session_id, port, session);
    });

    Ok(())
}

fn record_session_url(session_id: &str, url: &str) -> Result<Session> {
    let sessions_dir = store::sessions_dir()?;
    let lock = store::lock_session_for_update(&sessions_dir, session_id)?;
    let Some(mut session) = lock.load()? else {
        bail!("Agent session not found: {session_id}");
    };
    add_url(&mut session, url);
    lock.save(&session)?;
    drop(lock);
    Ok(session)
}

fn continue_after_session_recording(
    session_id: &str,
    result: Result<Session>,
    continue_review: impl FnOnce(Option<Session>),
) {
    let session = match result {
        Ok(session) => Some(session),
        Err(error) => {
            tracing::warn!(
                event = "agent.crit.add.session_record_failed",
                session = %session_id,
                error = %error,
            );
            eprintln!(
                "[armyknife] warning: failed to record crit review for the agent session: {error:#}"
            );
            None
        }
    };
    continue_review(session);
}

fn continue_add(args: &AddArgs, session_id: &str, port: u16, session: Option<Session>) {
    if let Some(session) = session.as_ref()
        && let Some(pane_id) = caller_pane::pane_id_for_session(session)
        && let Err(error) = pane::set_crit_url(&pane_id, &args.url)
    {
        tracing::warn!(
            event = "agent.crit.add.pane_sync_failed",
            pane = %pane_id,
            error = %error,
        );
        eprintln!("[armyknife] warning: failed to update the tmux crit pane option: {error}");
    }

    if let Err(error) = monitor::ensure_started(port, session.as_ref()) {
        tracing::warn!(
            event = "agent.crit.monitor.start_failed",
            session = %session_id,
            port,
            error = %error,
        );
        eprintln!("[armyknife] warning: failed to monitor crit review shutdown: {error:#}");
    }

    if session.is_none()
        && let Err(error) = display_unrecorded_review(args, port)
    {
        tracing::warn!(event = "agent.crit.add.review_display_failed", error = %error);
        eprintln!("[armyknife] warning: failed to display crit review: {error:#}");
    }

    let notification_result = session.as_ref().map_or_else(
        || send_unrecorded_notification(session_id, port),
        |session| send_notification(session, port),
    );
    if let Err(error) = notification_result {
        tracing::warn!(
            event = "agent.crit.notification.failed",
            session = %session_id,
            port,
            error = %error,
        );
        eprintln!("[armyknife] warning: failed to send crit notification: {error:#}");
    }
}

fn display_unrecorded_review(args: &AddArgs, port: u16) -> Result<()> {
    if let Some(pane_id) = caller_pane::resolve_caller_pane_id() {
        return tmux::open_crit_pane(tmux::CritPaneSpec {
            parent_pane_id: &pane_id,
            url: &args.url,
            port,
            title: " crit · review ",
            force_shpool_attach: false,
        });
    }
    open_url_in_default_browser(&args.url)
}

fn open_url_in_default_browser(url: &str) -> Result<()> {
    let mut opener = if cfg!(target_os = "macos") {
        ExternalTool::Open.command()
    } else {
        ExternalTool::XdgOpen.command()
    };
    let status = opener
        .arg(url)
        .status()
        .context("failed to open crit URL in the default browser")?;
    anyhow::ensure!(
        status.success(),
        "default browser opener exited with {status}"
    );
    Ok(())
}

fn send_unrecorded_notification(session_id: &str, port: u16) -> Result<()> {
    let notification = Notification::new(CRIT_TITLE, "Crit review requested")
        .with_subtitle(display_label(None, session_id))
        .with_content_image_url(format!("http://127.0.0.1:{port}/apple-touch-icon.png"));
    crate::infra::notification::send(&notification)
}

fn add_without_agent_session(args: &AddArgs, pane_id: &str) -> Result<()> {
    let port = parse_port(&args.url)?;
    let span = tracing::info_span!(
        "agent.crit.add",
        run_id = %short_run_id(),
        session = "none",
        port,
    );
    let _guard = span.enter();

    if let Err(error) = monitor::ensure_started(port, None) {
        tracing::warn!(
            event = "agent.crit.monitor.start_failed",
            port,
            error = %error,
        );
        eprintln!("[armyknife] warning: failed to monitor crit review shutdown: {error:#}");
    }

    let result = tmux::open_crit_pane(tmux::CritPaneSpec {
        parent_pane_id: pane_id,
        url: &args.url,
        port,
        title: " crit · review ",
        force_shpool_attach: false,
    });
    if let Err(error) = &result {
        tracing::warn!(event = "agent.crit.add.pane_open_failed", error = %error);
        return result;
    }

    tracing::info!(event = "agent.crit.add.pane_opened");
    result
}

fn open(args: &OpenArgs) -> Result<()> {
    let span = tracing::info_span!(
        "agent.crit.open",
        run_id = %short_run_id(),
        session = tracing::field::Empty,
    );
    let _guard = span.enter();
    let result = open_inner(args);
    if let Err(error) = &result {
        tracing::warn!(
            event = "agent.crit.open.failed",
            error = %error,
        );
        if args.parent_pid.is_some()
            && let Err(tmux_error) =
                tmux::run_tmux(&["display-message", "crit pane failed; see armyknife log"])
        {
            tracing::warn!(
                event = "agent.crit.open.failure_notice_failed",
                error = %tmux_error,
            );
        }
    }
    result
}

fn open_inner(args: &OpenArgs) -> Result<()> {
    if let Some(parent_pid) = args.parent_pid
        && !process::wait_for_process_exit(parent_pid, Duration::from_secs(300))
    {
        bail!("Timed out waiting for agent watch to exit");
    }

    let pane_id = args.pane.clone().or_else(tmux::current_pane_id_from_env);
    if args.session.is_none()
        && let Some(pane_id) = pane_id.as_deref()
        && toggle::close_if_crit_pane(pane_id, &toggle::LiveCritPaneRuntime)?
    {
        return Ok(());
    }

    let session_id = resolve_session_id(args)?;
    tracing::Span::current().record("session", tracing::field::display(&session_id));
    let session = store::load_session(&session_id)?
        .with_context(|| format!("Agent session not found: {session_id}"))?;
    let tmux_info = session
        .tmux_info
        .as_ref()
        .context("Agent session has no tmux pane")?;
    let target_pane_id = if args.session.is_some() {
        tmux_info.pane_id.as_str()
    } else {
        pane_id
            .as_deref()
            .context("Provide --session or --pane outside an agent tmux pane")?
    };

    let title = format!(
        " crit · {} · {} ",
        display_label(Some(&session), &session.session_id),
        repo_name(&session.cwd)
    );
    toggle::toggle_review_pane(
        target_pane_id,
        session.crit_urls.last().map(String::as_str),
        &title,
        &toggle::LiveCritPaneRuntime,
    )
}

fn close_crit_pane(pane_id: &str) -> Result<()> {
    tmux::close_crit_pane(pane_id)?;
    tracing::info!(event = "agent.crit.open.floating_pane_closed", pane = %pane_id);
    Ok(())
}

fn resolve_session_id(args: &OpenArgs) -> Result<String> {
    if let Some(session_id) = &args.session {
        return Ok(session_id.clone());
    }

    let pane_id = args.pane.clone().or_else(tmux::current_pane_id_from_env);
    let Some(pane_id) = pane_id else {
        bail!("Provide --session or --pane outside an agent tmux pane");
    };

    resolve_session_option(|option| tmux::get_pane_option(&pane_id, option))
        .with_context(|| format!("No agent session is bound to pane {pane_id}"))
}

fn send_notification(session: &Session, port: u16) -> Result<()> {
    let mut notification = Notification::new(CRIT_TITLE, repo_name(&session.cwd))
        .with_content_image_url(format!("http://127.0.0.1:{port}/apple-touch-icon.png"));
    let tmux_location = session
        .tmux_info
        .as_ref()
        .map(|info| format!("{}:{}", info.session_name, info.window_name));
    let subtitle = match tmux_location {
        Some(location) => format!(
            "{location} | {}",
            display_label(Some(session), &session.session_id)
        ),
        None => display_label(Some(session), &session.session_id),
    };
    notification = notification.with_subtitle(subtitle);

    if session.tmux_info.is_some() {
        let command = shlex::try_join([
            "a",
            "agent",
            "crit",
            "open",
            "--session",
            &session.session_id,
        ])
        .context("failed to quote the crit notification command")?;
        notification = notification.with_action(NotificationAction::new(command));
    }

    crate::infra::notification::send(&notification)
}

fn add_url(session: &mut Session, url: &str) {
    if let Some(position) = session
        .crit_urls
        .iter()
        .position(|existing| existing == url)
    {
        session.crit_urls.remove(position);
    }
    session.crit_urls.push(url.to_string());
}

pub(super) fn spawn_open_after_watch(session_id: &str) -> Result<()> {
    let executable = std::env::current_exe().context("failed to resolve armyknife executable")?;
    let parent_pid = std::process::id().to_string();
    let args = [
        "agent",
        "crit",
        "open",
        "--session",
        session_id,
        "--parent-pid",
        parent_pid.as_str(),
    ];
    process::spawn_detached(&executable, args, None, &[])
        .context("failed to start the crit pane after agent watch")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continues_review_flow_when_session_recording_fails() {
        let mut actions = Vec::new();
        let mut session_id_seen = None;

        continue_after_session_recording(
            "fixture-session",
            Err(anyhow::anyhow!("fixture record failure")),
            |session| {
                session_id_seen = session.map(|session| session.session_id);
                actions.extend(["display", "notification"]);
            },
        );

        assert_eq!(
            (session_id_seen, actions),
            (None, vec!["display", "notification"]),
        );
    }
}
