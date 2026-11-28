use super::{PaneInfo, Result, TmuxError, query_pane_value, run_tmux_output};
use crate::infra::process;

/// Lists the root process of every pane in the window that contains `pane_id`.
pub fn get_window_pane_pids(pane_id: &str) -> Result<Vec<u32>> {
    let output = run_tmux_output(&["list-panes", "-t", pane_id, "-F", "#{pane_pid}"])?;
    parse_pane_pids(&output)
}

fn parse_pane_pids(output: &str) -> Result<Vec<u32>> {
    output
        .lines()
        .map(|line| {
            line.parse().map_err(|_| {
                TmuxError::Internal(format!("tmux returned an invalid process ID `{line}`"))
            })
        })
        .collect()
}

/// Gets tmux pane information for a pane ID.
/// Returns None if the pane doesn't exist or tmux is unavailable.
pub fn get_pane_info_by_pane_id(pane_id: &str) -> Option<PaneInfo> {
    query_pane_value(
        pane_id,
        "#{session_name}\t#{window_name}\t#{window_index}\t#{pane_id}",
    )
    .and_then(|output| parse_pane_info_line(&output))
}

/// Gets tmux pane information for a process ID or one of its ancestors.
pub fn get_pane_info_by_pid(pid: u32) -> Option<PaneInfo> {
    get_pane_info_and_pid_by_pid(pid).map(|(pane_info, _)| pane_info)
}

/// Gets pane information and the matching pane process ID for a process.
pub(crate) fn get_pane_info_and_pid_by_pid(pid: u32) -> Option<(PaneInfo, u32)> {
    let output = run_tmux_output(&[
        "list-panes",
        "-a",
        "-F",
        "#{pane_pid}\t#{session_name}\t#{window_name}\t#{window_index}\t#{pane_id}",
    ])
    .ok()?;

    let pane_pids: std::collections::HashSet<u32> = output
        .lines()
        .filter_map(|line| line.split('\t').next()?.parse::<u32>().ok())
        .collect();

    let mut current_pid = pid;
    const MAX_DEPTH: usize = 20;

    for _ in 0..MAX_DEPTH {
        if pane_pids.contains(&current_pid) {
            let pane_info = output
                .lines()
                .find_map(|line| parse_pane_line_by_pid(line, current_pid))?;
            return Some((pane_info, current_pid));
        }

        match process::get_parent_pid(current_pid) {
            Some(parent_pid) if parent_pid != current_pid && parent_pid != 0 => {
                current_pid = parent_pid;
            }
            _ => break,
        }
    }

    None
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

fn parse_pane_line_by_pid(line: &str, target_pid: u32) -> Option<PaneInfo> {
    let mut parts = line.split('\t');

    let pane_pid = parts.next()?.parse::<u32>().ok()?;
    if pane_pid != target_pid {
        return None;
    }

    Some(PaneInfo {
        session_name: parts.next()?.to_string(),
        window_name: parts.next()?.to_string(),
        window_index: parts.next()?.parse::<u32>().ok()?,
        pane_id: parts.next()?.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use indoc::indoc;
    use rstest::rstest;

    use super::{parse_pane_line_by_pid, parse_pane_pids};
    use crate::infra::tmux::PaneInfo;

    #[rstest]
    #[case::every_pane(
        indoc! {"
            120
            340
        "},
        Ok(vec![120, 340])
    )]
    #[case::invalid_pid(
        indoc! {"
            120
            not-a-pid
        "},
        Err("tmux returned an invalid process ID `not-a-pid`".to_string())
    )]
    fn parses_every_pane_pid(
        #[case] output: &str,
        #[case] expected: std::result::Result<Vec<u32>, String>,
    ) {
        assert_eq!(
            parse_pane_pids(output).map_err(|error| error.to_string()),
            expected,
        );
    }

    #[rstest]
    #[case::standard_line(
        "12345\tmain\teditor\t0\t%0",
        12345,
        Some(PaneInfo {
            session_name: "main".to_string(),
            window_name: "editor".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        }),
    )]
    #[case::different_pid("12345\twork\tterminal\t1\t%5", 99999, None)]
    #[case::high_window_index(
        "54321\tsession\twindow\t99\t%123",
        54321,
        Some(PaneInfo {
            session_name: "session".to_string(),
            window_name: "window".to_string(),
            window_index: 99,
            pane_id: "%123".to_string(),
        }),
    )]
    #[case::session_with_spaces(
        "12345\tmy session\tmy window\t0\t%0",
        12345,
        Some(PaneInfo {
            session_name: "my session".to_string(),
            window_name: "my window".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        }),
    )]
    #[case::insufficient_parts("12345\tmain", 12345, None)]
    #[case::empty_line("", 12345, None)]
    fn parses_pane_by_pid(
        #[case] line: &str,
        #[case] target_pid: u32,
        #[case] expected: Option<PaneInfo>,
    ) {
        assert_eq!(parse_pane_line_by_pid(line, target_pid), expected);
    }
}
