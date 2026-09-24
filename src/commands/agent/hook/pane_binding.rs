//! Finds the tmux pane a hook event belongs to.
//!
//! The pane is normally reachable by walking the hook process's ancestry,
//! since the agent CLI runs inside the pane and spawns the hook as a child.
//! Codex breaks that link: when a shared `codex app-server` daemon is
//! listening on its control socket, the TUI in the pane is a thin client and
//! the agent loop -- hooks included -- runs inside the daemon, which launchd
//! started outside any pane's process tree. In that case, the session ID in
//! the hook payload is matched against the option written by the launcher.

use crate::commands::agent::pane::process::pane_has_live_agent_process;
use crate::commands::agent::types::{
    Engine, HookInput, TMUX_SESSION_OPTION, TMUX_SESSION_OPTION_LEGACY,
};
use crate::infra::process::ProcessSnapshot;
use crate::infra::tmux::{self, PaneInfo, PaneInfoWithOption};

/// Resolves the hook's pane through process ancestry or its explicit session binding.
pub(super) fn resolve(input: &HookInput) -> Option<PaneInfo> {
    if let Some(pane) = tmux::get_pane_info_by_pid(std::process::id()) {
        return Some(pane);
    }

    if input.engine != Engine::Codex {
        return None;
    }

    find_by_session_option(&input.session_id)
}

fn find_by_session_option(session_id: &str) -> Option<PaneInfo> {
    let panes = tmux::list_all_panes_with_option(TMUX_SESSION_OPTION, TMUX_SESSION_OPTION_LEGACY);
    if !panes
        .iter()
        .any(|pane| pane.option_value.as_deref() == Some(session_id))
    {
        return None;
    }

    let snapshot = ProcessSnapshot::capture()?;
    let pane_id = unique_live_pane_for_session(&panes, session_id, |pane_id| {
        tmux::get_pane_pid(pane_id).is_some_and(|pane_pid| {
            pane_has_live_agent_process(pane_pid, Engine::Codex, Some(&snapshot))
        })
    })?;

    tmux::get_pane_info_by_pane_id(pane_id)
}

fn unique_live_pane_for_session<'a>(
    panes: &'a [PaneInfoWithOption],
    session_id: &str,
    mut has_live_codex: impl FnMut(&str) -> bool,
) -> Option<&'a str> {
    let mut matches = panes
        .iter()
        .filter(|pane| pane.option_value.as_deref() == Some(session_id))
        .filter(|pane| has_live_codex(&pane.pane_id));
    let pane_id = &matches.next()?.pane_id;
    if matches.next().is_some() {
        tracing::warn!(
            event = "agent.hook.pane_binding.ambiguous",
            session_id,
            "multiple Codex panes carry this session ID"
        );
        return None;
    }

    Some(pane_id)
}

#[cfg(test)]
mod tests {
    use super::unique_live_pane_for_session;
    use crate::infra::tmux::PaneInfoWithOption;
    use rstest::{fixture, rstest};

    #[fixture]
    fn panes_with_session_options() -> Vec<PaneInfoWithOption> {
        vec![
            pane_with_option("%wrapper", "session-a"),
            pane_with_option("%shell", "session-a"),
            pane_with_option("%other", "session-b"),
        ]
    }

    fn pane_with_option(pane_id: &str, option_value: &str) -> PaneInfoWithOption {
        PaneInfoWithOption {
            session_name: "main".to_string(),
            window_index: 0,
            pane_index: 0,
            pane_id: pane_id.to_string(),
            option_value: Some(option_value.to_string()),
        }
    }

    #[rstest]
    #[case::wrapper_codex_and_stale_shell(vec!["%wrapper"], Some("%wrapper"))]
    #[case::stale_shell_only(Vec::new(), None)]
    #[case::multiple_live_codex_panes(vec!["%wrapper", "%shell"], None)]
    #[case::different_session_only(vec!["%other"], None)]
    fn unique_live_pane_cases(
        panes_with_session_options: Vec<PaneInfoWithOption>,
        #[case] live_pane_ids: Vec<&str>,
        #[case] expected: Option<&str>,
    ) {
        assert_eq!(
            unique_live_pane_for_session(&panes_with_session_options, "session-a", |pane_id| {
                live_pane_ids.contains(&pane_id)
            }),
            expected,
        );
    }
}
