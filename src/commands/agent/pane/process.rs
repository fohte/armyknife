//! Detects whether a tmux pane's process tree already has a live agent
//! (Claude Code or Codex) process running.

use std::collections::HashSet;

use crate::commands::agent::types::Engine;
use crate::infra::process::{ProcessInfo, ProcessSnapshot};

/// Bound for the descendant walk that resolves whether an agent process
/// is already running in a pane. Same value `auto_compact::schedule` and
/// `sweep` use; a shell hosting an agent has at most a handful of children.
const MAX_DESCENDANT_NODES: usize = 64;

/// Returns whether `pane_pid`'s process tree -- the pane's own process or
/// any descendant -- currently has a running process for `engine`.
///
/// Fails closed: an unavailable `snapshot` (e.g. `ps` failed) is treated as
/// "has a live process" so callers do not mistake an unknown process tree for
/// an idle pane. Callers that need positive confirmation must handle snapshot
/// failure before calling this function.
pub fn pane_has_live_agent_process(
    pane_pid: u32,
    engine: Engine,
    snapshot: Option<&ProcessSnapshot>,
) -> bool {
    let Some(snapshot) = snapshot else {
        return true;
    };
    snapshot
        .find_self_or_descendant_by_command(pane_pid, engine.process_name(), MAX_DESCENDANT_NODES)
        .is_some()
}

/// Returns whether another agent process is above the current agent in the
/// hook's ancestry. Returns `None` if any ancestor cannot be inspected.
pub(crate) fn has_parent_agent_process(pid: u32) -> Option<bool> {
    has_parent_agent_process_with(pid, crate::infra::process::get_process_info)
}

fn has_parent_agent_process_with(
    pid: u32,
    mut get_process_info: impl FnMut(u32) -> Option<ProcessInfo>,
) -> Option<bool> {
    let mut current_pid = pid;
    let mut visited = HashSet::new();
    let mut agent_ancestor_count = 0;

    loop {
        if !visited.insert(current_pid) {
            break;
        }
        let info = get_process_info(current_pid)?;
        if current_pid != pid
            && [Engine::Claude, Engine::Codex]
                .into_iter()
                .any(|engine| info.comm == engine.process_name())
        {
            agent_ancestor_count += 1;
        }
        if info.parent_pid == 0 || info.parent_pid == current_pid {
            break;
        }
        current_pid = info.parent_pid;
    }

    Some(agent_ancestor_count > 1)
}

#[cfg(test)]
mod tests {
    use super::has_parent_agent_process_with;
    use crate::infra::process::ProcessInfo;
    use indoc::indoc;
    use rstest::rstest;
    use std::collections::HashMap;

    #[rstest]
    #[case::nested_under_claude(
        indoc! {"
            1 0 init
            100 1 init
            200 100 claude
            300 200 claude
            400 300 hook
        "},
        400,
        Some(true),
    )]
    #[case::nested_under_codex(
        indoc! {"
            1 0 init
            100 1 init
            200 100 claude
            300 200 codex
            400 300 hook
        "},
        400,
        Some(true),
    )]
    #[case::direct_agent_process(
        indoc! {"
            1 0 init
            100 1 init
            200 100 claude
            300 200 hook
        "},
        300,
        Some(false),
    )]
    #[case::ordinary_shell_ancestor(
        indoc! {"
            1 0 init
            100 1 init
            200 100 zsh
            300 200 hook
        "},
        300,
        Some(false),
    )]
    #[case::unavailable_ancestor(
        indoc! {"
            300 200 hook
        "},
        300,
        None,
    )]
    fn has_parent_agent_process_cases(
        #[case] process_table: &str,
        #[case] pid: u32,
        #[case] expected: Option<bool>,
    ) {
        let processes = parse_process_table(process_table);
        assert_eq!(
            has_parent_agent_process_with(pid, |pid| processes.get(&pid).cloned()),
            expected,
        );
    }

    fn parse_process_table(text: &str) -> HashMap<u32, ProcessInfo> {
        text.lines()
            .filter_map(|line| {
                let mut columns = line.split_whitespace();
                let pid = columns.next()?.parse::<u32>().ok()?;
                let parent_pid = columns.next()?.parse::<u32>().ok()?;
                let comm = columns.next()?.to_string();
                Some((pid, ProcessInfo { parent_pid, comm }))
            })
            .collect()
    }
}
