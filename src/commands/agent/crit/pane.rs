use crate::commands::agent::types::TMUX_CRIT_OPTION;
use crate::infra::tmux;

use super::url::parse_port;

pub(super) fn set_crit_url(pane_id: &str, url: &str) -> tmux::Result<()> {
    tmux::set_pane_option(pane_id, TMUX_CRIT_OPTION, url)?;
    rerun_window_layout_hook(pane_id)
}

pub(super) fn restore_crit_url(
    pane_id: &str,
    removed_port: u16,
    latest_url: Option<&str>,
) -> tmux::Result<()> {
    if let Some(url) = latest_url {
        tmux::set_pane_option(pane_id, TMUX_CRIT_OPTION, url)?;
        return rerun_window_layout_hook(pane_id);
    }

    let current_port = tmux::get_pane_option(pane_id, TMUX_CRIT_OPTION)
        .as_deref()
        .and_then(|url| parse_port(url).ok());
    if current_port == Some(removed_port) {
        tmux::run_tmux(&["set-option", "-p", "-u", "-t", pane_id, TMUX_CRIT_OPTION])?;
        rerun_window_layout_hook(pane_id)?;
    }
    Ok(())
}

fn rerun_window_layout_hook(pane_id: &str) -> tmux::Result<()> {
    tmux::run_tmux(&["set-hook", "-R", "-t", pane_id, "window-layout-changed"])
}
