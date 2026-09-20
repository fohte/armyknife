//! Finds the tmux pane a hook event belongs to.
//!
//! The pane is normally reachable by walking the hook process's ancestry,
//! since the agent CLI runs inside the pane and spawns the hook as a child.
//! Codex breaks that link: when a shared `codex app-server` daemon is
//! listening on its control socket, the TUI in the pane is a thin client and
//! the agent loop -- hooks included -- runs inside the daemon, which launchd
//! started outside any pane's process tree. Such a hook has no ancestry path
//! to the pane and carries nothing pane-specific in its payload or
//! environment, so the pane has to be recognized by the client it is running.

use crate::commands::agent::types::HookInput;
use crate::infra::tmux::{self, PaneInfo, PaneProcess};

/// Resolves the pane that `input`'s session is running in, or `None` when it
/// cannot be determined unambiguously.
pub(super) fn resolve(input: &HookInput) -> Option<PaneInfo> {
    if let Some(pane_info) = tmux::get_pane_info_by_pid(std::process::id()) {
        return Some(pane_info);
    }

    unique_pane_running(
        &tmux::list_pane_processes(),
        input.engine.process_name(),
        &input.cwd.to_string_lossy(),
    )
}

/// Picks the single pane running `command` in `cwd`.
///
/// Returns `None` when several panes match: they are indistinguishable from
/// here, and binding the session to the wrong one would make `a agent resume`
/// relaunch someone else's session in this pane.
fn unique_pane_running(panes: &[PaneProcess], command: &str, cwd: &str) -> Option<PaneInfo> {
    let mut matches = panes
        .iter()
        .filter(|pane| pane.current_command == command && pane.current_path == cwd);

    let pane = matches.next()?;
    matches.next().is_none().then(|| pane.info.clone())
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::commands::agent::types::Engine;

    fn pane(pane_id: &str, current_command: &str, current_path: &str) -> PaneProcess {
        PaneProcess {
            info: PaneInfo {
                session_name: "fohte/armyknife".to_string(),
                window_name: "main".to_string(),
                window_index: 1,
                pane_id: pane_id.to_string(),
            },
            current_command: current_command.to_string(),
            current_path: current_path.to_string(),
        }
    }

    fn pane_info(pane_id: &str) -> PaneInfo {
        PaneInfo {
            session_name: "fohte/armyknife".to_string(),
            window_name: "main".to_string(),
            window_index: 1,
            pane_id: pane_id.to_string(),
        }
    }

    #[rstest]
    #[case::single_match(
        vec![pane("%1", "zsh", "/repo"), pane("%2", "codex", "/repo")],
        Some(pane_info("%2")),
    )]
    #[case::other_command_in_same_dir(
        vec![pane("%1", "nvim", "/repo")],
        None,
    )]
    #[case::same_command_in_other_dir(
        vec![pane("%1", "codex", "/other")],
        None,
    )]
    #[case::ambiguous_between_two_panes(
        vec![pane("%1", "codex", "/repo"), pane("%2", "codex", "/repo")],
        None,
    )]
    #[case::no_panes(vec![], None)]
    fn selects_unambiguous_pane(
        #[case] panes: Vec<PaneProcess>,
        #[case] expected: Option<PaneInfo>,
    ) {
        assert_eq!(unique_pane_running(&panes, "codex", "/repo"), expected);
    }

    #[test]
    fn matches_the_engine_running_in_the_pane() {
        let panes = vec![pane("%1", "claude", "/repo"), pane("%2", "codex", "/repo")];

        assert_eq!(
            unique_pane_running(&panes, Engine::Codex.process_name(), "/repo"),
            Some(pane_info("%2")),
        );
    }
}
