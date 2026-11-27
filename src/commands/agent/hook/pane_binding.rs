//! Finds the tmux pane a hook event belongs to.
//!
//! The pane is normally reachable by walking the hook process's ancestry,
//! since the agent CLI runs inside the pane and spawns the hook as a child.
//! Codex breaks that link: when a shared `codex app-server` daemon is
//! listening on its control socket, the TUI in the pane is a thin client and
//! the agent loop -- hooks included -- runs inside the daemon, which launchd
//! started outside any pane's process tree. In that case, the session ID in
//! the hook payload is matched against the option written by the launcher.

use std::path::Path;

use crate::commands::agent::pane::process::{
    has_parent_agent_process, pane_has_live_agent_process,
};
use crate::commands::agent::store;
use crate::commands::agent::types::{
    Engine, HookInput, TMUX_SESSION_OPTION, TMUX_SESSION_OPTION_LEGACY,
};
use crate::infra::process::ProcessSnapshot;
use crate::infra::tmux::{self, PaneInfo, PaneInfoWithOption};

/// Resolves the hook's pane through process ancestry or its explicit session binding.
pub(super) fn resolve(input: &HookInput) -> Option<PaneInfo> {
    resolve_with(
        input.engine,
        &input.session_id,
        std::process::id(),
        tmux::get_pane_info_and_pid_by_pid,
        has_parent_agent_process,
        find_by_session_option,
    )
}

fn resolve_with(
    engine: Engine,
    session_id: &str,
    pid: u32,
    get_pane_info: impl FnOnce(u32) -> Option<(PaneInfo, u32)>,
    has_parent_agent: impl FnOnce(u32, u32) -> Option<bool>,
    find_by_session: impl FnOnce(&str) -> Option<PaneInfo>,
) -> Option<PaneInfo> {
    if let Some((pane, pane_pid)) = get_pane_info(pid) {
        return match has_parent_agent(pid, pane_pid) {
            Some(false) => Some(pane),
            Some(true) | None => None,
        };
    }

    if engine == Engine::Codex {
        return find_by_session(session_id);
    }

    None
}

/// Detects a reused pane so its inherited launch environment cannot pin a new session.
pub(super) fn has_other_session_on_pane(
    sessions_dir: &Path,
    session_id: &str,
    pane_id: &str,
) -> bool {
    let Ok(sessions) = store::list_all_sessions_from(sessions_dir) else {
        return false;
    };

    sessions.into_iter().any(|session| {
        session.session_id != session_id
            && session
                .tmux_info
                .as_ref()
                .is_some_and(|info| info.pane_id == pane_id)
    })
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
    use super::{resolve_with, unique_live_pane_for_session};
    use crate::commands::agent::types::Engine;
    use crate::infra::tmux::{PaneInfo, PaneInfoWithOption};
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

    fn pane_info() -> PaneInfo {
        PaneInfo {
            session_name: "session-main".to_string(),
            window_name: "window-main".to_string(),
            window_index: 0,
            pane_id: "%12".to_string(),
        }
    }

    #[rstest]
    #[case::nested_agent_is_not_bound(true, None)]
    #[case::direct_agent_keeps_pane_binding(false, Some(pane_info()))]
    fn ancestry_pane_binding_cases(
        #[case] has_parent_agent: bool,
        #[case] expected: Option<PaneInfo>,
    ) {
        let mut ancestry_arguments = None;
        let result = resolve_with(
            Engine::Claude,
            "session-example",
            123,
            |_| Some((pane_info(), 100)),
            |hook_pid, pane_pid| {
                ancestry_arguments = Some((hook_pid, pane_pid));
                Some(has_parent_agent)
            },
            |_| None,
        );

        assert_eq!((result, ancestry_arguments), (expected, Some((123, 100))),);
    }

    #[rstest]
    #[case::snapshot_failure_is_unresolved(None)]
    #[case::nested_agent_is_unresolved(Some(true))]
    fn ancestry_pane_binding_requires_a_known_direct_agent(#[case] has_parent_agent: Option<bool>) {
        assert_eq!(
            resolve_with(
                Engine::Claude,
                "session-example",
                123,
                |_| Some((pane_info(), 100)),
                |_, _| has_parent_agent,
                |_| None,
            ),
            None,
        );
    }

    #[test]
    fn codex_without_ancestry_pane_uses_session_option_lookup() {
        let expected = pane_info();
        assert_eq!(
            resolve_with(
                Engine::Codex,
                "session-example",
                123,
                |_| None,
                |_, _| Some(false),
                |session_id| (session_id == "session-example").then(|| expected.clone()),
            ),
            Some(expected),
        );
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
