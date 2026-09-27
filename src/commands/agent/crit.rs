//! Link crit reviews to agent sessions and open them in tmux popups.

mod monitor;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use std::path::Path;
use std::thread;
use std::time::Duration;

use super::store;
use super::types::{Session, TMUX_CRIT_OPTION};
use crate::infra::external_tool::ExternalTool;
use crate::infra::notification::{Notification, NotificationAction};
use crate::infra::tmux;
use crate::shared::env_var::EnvVars;

const CRIT_TITLE: &str = "■ crit - Review requested";

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum CritCommands {
    /// Associate a crit review URL with the current agent session.
    Add(AddArgs),

    /// Open the latest crit review for a session or pane in a tmux popup.
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
    pub after_watch: bool,
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
        let status = ExternalTool::Open
            .command()
            .arg(&args.url)
            .status()
            .context("failed to open crit URL in the default browser")?;
        anyhow::ensure!(status.success(), "open exited with {status}");
        return Ok(());
    };

    let port = parse_port(&args.url)?;
    let sessions_dir = store::sessions_dir()?;
    let lock = store::lock_session_for_update(&sessions_dir, &session_id)?;
    let Some(mut session) = lock.load()? else {
        bail!("Agent session not found: {session_id}");
    };
    add_url(&mut session, &args.url);
    lock.save(&session)?;

    if let Some(tmux_info) = &session.tmux_info {
        tmux::set_pane_option(&tmux_info.pane_id, TMUX_CRIT_OPTION, &args.url)
            .context("failed to set crit URL on the tmux pane")?;
        rerun_window_layout_hook(&tmux_info.pane_id)
            .context("failed to re-evaluate the tmux pane border")?;
    }
    drop(lock);

    if let Err(error) = monitor::ensure_started(port, &session) {
        tracing::warn!(session_id, port, %error, "failed to start crit lifecycle monitor");
        eprintln!("[armyknife] warning: failed to monitor crit review shutdown: {error:#}");
    }

    if let Err(error) = send_notification(&session, port) {
        tracing::warn!(session_id, port, %error, "failed to send crit notification");
        eprintln!("[armyknife] warning: failed to send crit notification: {error:#}");
    }

    Ok(())
}

fn open(args: &OpenArgs) -> Result<()> {
    if args.after_watch {
        thread::sleep(Duration::from_secs(1));
    }
    let session_id = resolve_session_id(args)?;
    let session = store::load_session(&session_id)?
        .with_context(|| format!("Agent session not found: {session_id}"))?;
    let url = session
        .crit_urls
        .last()
        .context("No crit review is associated with this agent session")?;
    let port = parse_port(url)?;
    let tmux_info = session
        .tmux_info
        .as_ref()
        .context("Agent session has no tmux pane")?;

    tmux::focus_pane(&tmux_info.pane_id).context("failed to focus the agent tmux pane")?;
    display_popup(&session, url, port, &tmux_info.pane_id)
}

fn resolve_session_id(args: &OpenArgs) -> Result<String> {
    if let Some(session_id) = &args.session {
        return Ok(session_id.clone());
    }

    let pane_id = args.pane.clone().or_else(tmux::current_pane_id_from_env);
    let Some(pane_id) = pane_id else {
        bail!("Provide --session or --pane outside an agent tmux pane");
    };

    store::list_all_sessions()?
        .into_iter()
        .find(|session| {
            session
                .tmux_info
                .as_ref()
                .is_some_and(|info| info.pane_id == pane_id)
        })
        .map(|session| session.session_id)
        .with_context(|| format!("No agent session is associated with pane {pane_id}"))
}

fn display_popup(session: &Session, url: &str, port: u16, pane_id: &str) -> Result<()> {
    let shpool = tool_path(ExternalTool::Shpool)?;
    let terminal_browser = tool_path(ExternalTool::TerminalBrowser)?;
    let shpool_session = format!("crit-{port}");
    let inner_command = shell_join([terminal_browser.as_str(), "open", url])?;
    let command = shell_join([
        shpool.as_str(),
        "attach",
        "-c",
        inner_command.as_str(),
        shpool_session.as_str(),
    ])?;

    let label = display_label(session);
    let repo = repo_name(&session.cwd);
    let title = tmux_title(&format!(" crit · {label} · {repo} "));
    tmux::run_tmux(&[
        "display-popup",
        "-E",
        "-w",
        "90%",
        "-h",
        "90%",
        "-b",
        "rounded",
        "-S",
        "fg=colour98",
        "-t",
        pane_id,
        "-T",
        &title,
        &command,
    ])?;
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
        Some(location) => format!("{location} | {}", display_label(session)),
        None => display_label(session),
    };
    notification = notification.with_subtitle(subtitle);

    if session.tmux_info.is_some() {
        let command = shell_join([
            "a",
            "agent",
            "crit",
            "open",
            "--session",
            &session.session_id,
        ])?;
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

fn parse_port(url: &str) -> Result<u16> {
    let (_, rest) = url
        .split_once("://")
        .with_context(|| format!("Invalid crit review URL: {url}"))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let (_, port) = authority
        .rsplit_once(':')
        .with_context(|| format!("Crit review URL has no port: {url}"))?;
    let port = port
        .parse::<u16>()
        .with_context(|| format!("Invalid port in crit review URL: {url}"))?;
    anyhow::ensure!(port != 0, "Crit review URL has an invalid port: {url}");
    Ok(port)
}

fn rerun_window_layout_hook(pane_id: &str) -> crate::infra::tmux::Result<()> {
    tmux::run_tmux(&["set-hook", "-R", "-t", pane_id, "window-layout-changed"])
}

fn repo_name(cwd: &Path) -> String {
    crate::infra::git::get_repo_root_in(cwd)
        .ok()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| cwd.to_path_buf())
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown")
        .to_string()
}

fn display_label(session: &Session) -> String {
    session
        .label
        .as_deref()
        .filter(|label| !label.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| short_id(&session.session_id))
}

fn short_id(session_id: &str) -> String {
    session_id.chars().take(8).collect()
}

fn tmux_title(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .replace('#', "##")
}

fn tool_path(tool: ExternalTool) -> Result<String> {
    tool.resolve_path()
        .with_context(|| format!("{} executable not found on PATH", tool.name()))?
        .to_str()
        .map(str::to_string)
        .with_context(|| format!("{} executable path is not valid UTF-8", tool.name()))
}

fn shell_join<'a>(args: impl IntoIterator<Item = &'a str>) -> Result<String> {
    args.into_iter()
        .map(|arg| {
            shlex::try_quote(arg)
                .map(|quoted| quoted.into_owned())
                .map_err(Into::into)
        })
        .collect::<Result<Vec<_>>>()
        .map(|args| args.join(" "))
}
