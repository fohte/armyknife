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

use crate::commands::agent::store;
use crate::commands::agent::types::{
    Engine, HookInput, SessionStatus, TMUX_SESSION_OPTION, TMUX_SESSION_OPTION_LEGACY,
};
use crate::infra::tmux::{self, PaneInfo, PaneProcess};

/// Resolves the pane that `input`'s session is running in, or `None` when it
/// cannot be determined unambiguously.
pub(super) fn resolve(input: &HookInput) -> Option<PaneInfo> {
    if let Some(pane_info) = tmux::get_pane_info_by_pid(std::process::id()) {
        return Some(pane_info);
    }

    // Recognizing the pane by what it runs is a guess, so it is confined to the
    // engine whose ancestry is known to be severable. For any other engine a
    // failed ancestry walk means something unforeseen, and guessing there would
    // overwrite a correct binding with a plausible-looking wrong one.
    if input.engine != Engine::Codex {
        return None;
    }

    unique_pane_running(
        &tmux::list_pane_processes(TMUX_SESSION_OPTION, TMUX_SESSION_OPTION_LEGACY),
        input.engine.process_name(),
        &input.cwd.to_string_lossy(),
        &input.session_id,
        session_is_alive,
    )
}

/// Whether the session bound to a pane is still occupying it.
///
/// A session whose file is gone left no trace of where it ran, and an ended or
/// paused one no longer has a process in the pane, so in both cases the pane is
/// free to be rebound.
fn session_is_alive(session_id: &str) -> bool {
    store::load_session(session_id)
        .ok()
        .flatten()
        .is_some_and(|session| {
            !matches!(session.status, SessionStatus::Ended | SessionStatus::Paused)
        })
}

/// Picks the single pane running `command` in `cwd` that `session_id` may claim.
///
/// Returns `None` when several panes match: they are indistinguishable from
/// here, and binding the session to the wrong one would make `a agent resume`
/// relaunch someone else's session in this pane. A pane another live session is
/// already bound to is not a candidate at all -- a pane-less session (`codex
/// exec` from a script, say) sharing the cwd would otherwise steal the binding
/// from the session the user is actually typing into.
fn unique_pane_running(
    panes: &[PaneProcess],
    command: &str,
    cwd: &str,
    session_id: &str,
    is_alive: impl Fn(&str) -> bool,
) -> Option<PaneInfo> {
    let mut matches = panes
        .iter()
        .filter(|pane| pane.current_command == command && pane.current_path == cwd)
        .filter(|pane| match pane.option_value.as_deref() {
            Some(bound) => bound == session_id || !is_alive(bound),
            None => true,
        });

    let pane = matches.next()?;
    matches.next().is_none().then(|| pane.info.clone())
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    const SESSION_ID: &str = "01a0bdf2-0000-7000-8000-000000000001";
    const OTHER_SESSION_ID: &str = "01a0bdec-0000-7000-8000-000000000002";

    fn pane(pane_id: &str, current_command: &str, current_path: &str) -> PaneProcess {
        PaneProcess {
            info: pane_info(pane_id),
            current_command: current_command.to_string(),
            current_path: current_path.to_string(),
            option_value: None,
        }
    }

    fn bound_pane(
        pane_id: &str,
        current_command: &str,
        current_path: &str,
        bound_session_id: &str,
    ) -> PaneProcess {
        PaneProcess {
            option_value: Some(bound_session_id.to_string()),
            ..pane(pane_id, current_command, current_path)
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

    /// Every session the tests bind a pane to is alive unless a case says
    /// otherwise, so a pane that gets picked was picked despite being claimed.
    fn all_alive(_: &str) -> bool {
        true
    }

    #[rstest]
    #[case::single_match(
        vec![pane("%1", "zsh", "/repo"), pane("%2", "codex", "/repo")],
        Some(pane_info("%2")),
    )]
    #[case::other_command_in_same_dir(vec![pane("%1", "nvim", "/repo")], None)]
    #[case::same_command_in_other_dir(vec![pane("%1", "codex", "/other")], None)]
    #[case::ambiguous_between_two_panes(
        vec![pane("%1", "codex", "/repo"), pane("%2", "codex", "/repo")],
        None,
    )]
    #[case::no_panes(vec![], None)]
    #[case::already_bound_to_this_session(
        vec![bound_pane("%1", "codex", "/repo", SESSION_ID)],
        Some(pane_info("%1")),
    )]
    #[case::bound_to_another_live_session(
        vec![bound_pane("%1", "codex", "/repo", OTHER_SESSION_ID)],
        None,
    )]
    #[case::live_neighbour_disambiguates(
        vec![
            bound_pane("%1", "codex", "/repo", OTHER_SESSION_ID),
            pane("%2", "codex", "/repo"),
        ],
        Some(pane_info("%2")),
    )]
    fn selects_unambiguous_pane(
        #[case] panes: Vec<PaneProcess>,
        #[case] expected: Option<PaneInfo>,
    ) {
        assert_eq!(
            unique_pane_running(&panes, "codex", "/repo", SESSION_ID, all_alive),
            expected,
        );
    }

    #[test]
    fn reclaims_a_pane_whose_session_has_gone() {
        let panes = vec![bound_pane("%1", "codex", "/repo", OTHER_SESSION_ID)];

        assert_eq!(
            unique_pane_running(&panes, "codex", "/repo", SESSION_ID, |_| false),
            Some(pane_info("%1")),
        );
    }

    #[test]
    fn matches_the_engine_running_in_the_pane() {
        let panes = vec![pane("%1", "claude", "/repo"), pane("%2", "codex", "/repo")];

        assert_eq!(
            unique_pane_running(
                &panes,
                Engine::Codex.process_name(),
                "/repo",
                SESSION_ID,
                all_alive,
            ),
            Some(pane_info("%2")),
        );
    }
}
