use super::types::TmuxInfo;
use crate::infra::tmux;

/// Gets tmux pane information for a given TTY device.
/// Returns None if not running in tmux or if the TTY is not found.
pub fn get_tmux_info_from_tty(tty: &str) -> Option<TmuxInfo> {
    tmux::get_pane_info_by_tty(tty).map(|info| TmuxInfo {
        session_name: info.session_name,
        window_name: info.window_name,
        window_index: info.window_index,
        pane_id: info.pane_id,
    })
}
