use super::{PaneInfo, query_pane_value};

/// Gets tmux pane information for a pane ID.
/// Returns None if the pane doesn't exist or tmux is unavailable.
pub fn get_pane_info_by_pane_id(pane_id: &str) -> Option<PaneInfo> {
    query_pane_value(
        pane_id,
        "#{session_name}\t#{window_name}\t#{window_index}\t#{pane_id}",
    )
    .and_then(|output| parse_pane_info_line(&output))
}

fn parse_pane_info_line(line: &str) -> Option<PaneInfo> {
    let mut parts = line.split('\t');

    Some(PaneInfo {
        session_name: parts.next()?.to_string(),
        window_name: parts.next()?.to_string(),
        window_index: parts.next()?.parse::<u32>().ok()?,
        pane_id: parts.next()?.to_string(),
    })
}
