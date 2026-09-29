use anyhow::{Context, Result};

use crate::infra::external_tool::ExternalTool;

const CRIT_PANE_OPTION: &str = "@armyknife-crit-pane";

pub(crate) struct CritPaneSpec<'a> {
    pub(crate) parent_pane_id: &'a str,
    pub(crate) url: &'a str,
    pub(crate) port: u16,
    pub(crate) title: &'a str,
}

pub(crate) fn is_crit_pane(pane_id: &str) -> bool {
    super::get_pane_option(pane_id, CRIT_PANE_OPTION).is_some()
}

pub(crate) fn find_crit_pane_for_parent(parent_pane_id: &str) -> super::Result<Option<String>> {
    super::find_pane_with_option_value(parent_pane_id, CRIT_PANE_OPTION, parent_pane_id)
}

pub(crate) fn close_crit_pane(pane_id: &str) -> super::Result<()> {
    super::run_tmux(&["kill-pane", "-t", pane_id])
}

pub(crate) fn open_crit_pane(spec: CritPaneSpec<'_>) -> Result<()> {
    let shpool = tool_path(ExternalTool::Shpool)?;
    let terminal_browser = tool_path(ExternalTool::TerminalBrowser)?;
    let shpool_session = format!("crit-{}", spec.port);
    let inner_command = shlex::try_join([terminal_browser.as_str(), "open", spec.url])
        .context("failed to quote terminal-browser command")?;
    let shpool_command = shlex::try_join([
        shpool.as_str(),
        "attach",
        "-c",
        inner_command.as_ref(),
        shpool_session.as_str(),
    ])
    .context("failed to quote shpool command")?;
    // shpool does not restore modifyOtherKeys across session attaches.
    let command = format!("printf '\\033[>4;2m' && exec {shpool_command}");

    let title = tmux_title(spec.title);
    let crit_pane_id = super::run_tmux_output(&[
        "new-pane",
        "-P",
        "-F",
        "#{pane_id}",
        "-x",
        "90%",
        "-y",
        "90%",
        "-X",
        "5%",
        "-Y",
        "5%",
        "-S",
        "fg=colour98",
        "-t",
        spec.parent_pane_id,
        command.as_str(),
    ])?;
    super::set_pane_option(&crit_pane_id, CRIT_PANE_OPTION, spec.parent_pane_id)?;
    super::run_tmux(&["select-pane", "-T", &title, "-t", &crit_pane_id])?;
    Ok(())
}

fn tool_path(tool: ExternalTool) -> Result<String> {
    tool.resolve_path()
        .with_context(|| format!("{} executable not found on PATH", tool.name()))?
        .to_str()
        .map(str::to_string)
        .with_context(|| format!("{} executable path is not valid UTF-8", tool.name()))
}

fn tmux_title(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .replace('#', "##")
}
