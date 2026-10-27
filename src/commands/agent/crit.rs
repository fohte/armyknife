//! Link crit reviews to agent sessions and open them in tmux floating panes.

mod monitor;
pub(super) mod toggle;
mod url;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use std::time::Duration;

use super::review_queue::{self, LiveQueueRuntime};
use super::store;
use super::types::Session;
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

    /// Show the next review (editor reviews, then crit) of an agent pane in a tmux floating pane.
    Open(OpenArgs),

    /// Close the displayed review of an agent pane. The review keeps running.
    Close(CloseArgs),

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

#[derive(Args, Clone, PartialEq, Eq)]
pub struct CloseArgs {
    /// Tmux pane ID whose displayed review should be closed.
    #[arg(long)]
    pub pane: Option<String>,
}

pub fn run(command: &CritCommands) -> Result<()> {
    match command {
        CritCommands::Add(args) => add(args),
        CritCommands::Open(args) => open(args),
        CritCommands::Close(args) => close(args),
        CritCommands::Monitor(args) => monitor::run(args),
    }
}

fn add(args: &AddArgs) -> Result<()> {
    let Some(session_id) = EnvVars::load().own_session_id() else {
        if let Some(pane_id) = caller_pane::resolve_caller_pane_id() {
            return add_without_agent_session(args, &pane_id);
        }

        let mut opener = if cfg!(target_os = "macos") {
            ExternalTool::Open.command()
        } else {
            ExternalTool::XdgOpen.command()
        };
        let status = opener
            .arg(&args.url)
            .status()
            .context("failed to open crit URL in the default browser")?;
        anyhow::ensure!(
            status.success(),
            "default browser opener exited with {status}"
        );
        return Ok(());
    };

    let port = parse_port(&args.url)?;
    let span = tracing::info_span!(
        "agent.crit.add",
        run_id = %short_run_id(),
        session = %session_id,
        port,
    );
    let _guard = span.enter();
    let sessions_dir = store::sessions_dir()?;
    let lock = store::lock_session_for_update(&sessions_dir, &session_id)?;
    let Some(mut session) = lock.load()? else {
        bail!("Agent session not found: {session_id}");
    };
    add_url(&mut session, &args.url);
    lock.save(&session)?;
    drop(lock);
    tracing::info!(event = "agent.crit.add.registered");

    if let Some(pane_id) = caller_pane::pane_id_for_session(&session)
        && let Err(error) =
            review_queue::refresh(&LiveQueueRuntime::for_session(&session), &pane_id)
    {
        tracing::warn!(
            event = "agent.crit.add.pane_sync_failed",
            pane = %pane_id,
            error = %error,
        );
        eprintln!("[armyknife] warning: failed to update the review queue: {error:#}");
    }
    if let Err(error) = monitor::ensure_started(port, Some(&session)) {
        tracing::warn!(
            event = "agent.crit.monitor.start_failed",
            session = %session_id,
            port,
            error = %error,
        );
        eprintln!("[armyknife] warning: failed to monitor crit review shutdown: {error:#}");
    }

    if let Err(error) = send_notification(&session, port) {
        tracing::warn!(
            event = "agent.crit.notification.failed",
            session = %session_id,
            port,
            error = %error,
        );
        eprintln!("[armyknife] warning: failed to send crit notification: {error:#}");
    }

    Ok(())
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

    if let Some(session_id) = &args.session {
        tracing::Span::current().record("session", tracing::field::display(session_id));
        let session = store::load_session(session_id)?
            .with_context(|| format!("Agent session not found: {session_id}"))?;
        let tmux_info = session
            .tmux_info
            .as_ref()
            .context("Agent session has no tmux pane")?;
        return review_queue::show_crit(
            &LiveQueueRuntime::for_session(&session),
            &tmux_info.pane_id,
        );
    }

    let parent_pane_id = resolve_parent_pane_id(args.pane.as_deref())?;
    let runtime = queue_runtime_for_pane(&parent_pane_id)?;
    review_queue::cycle(&runtime, &parent_pane_id)
}

fn close(args: &CloseArgs) -> Result<()> {
    let parent_pane_id = resolve_parent_pane_id(args.pane.as_deref())?;
    review_queue::hide(&queue_runtime_for_pane(&parent_pane_id)?, &parent_pane_id)
}

fn queue_runtime_for_pane(parent_pane_id: &str) -> Result<LiveQueueRuntime> {
    LiveQueueRuntime::for_pane(parent_pane_id)
        .with_context(|| format!("No agent session is bound to pane {parent_pane_id}"))
}

/// The key bindings run in whichever pane has focus, which may be a review
/// pane floating over the agent pane.
fn resolve_parent_pane_id(pane: Option<&str>) -> Result<String> {
    let pane_id = pane
        .map(str::to_string)
        .or_else(tmux::current_pane_id_from_env)
        .context("Provide --session or --pane outside an agent tmux pane")?;
    Ok(tmux::get_crit_pane_parent_id(&pane_id)
        .or_else(|| tmux::get_editor_pane_parent_id(&pane_id))
        .unwrap_or(pane_id))
}

fn close_crit_pane(pane_id: &str) -> Result<()> {
    tmux::close_crit_pane(pane_id)?;
    tracing::info!(event = "agent.crit.open.floating_pane_closed", pane = %pane_id);
    Ok(())
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
