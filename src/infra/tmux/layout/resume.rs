use crate::commands::agent::types::{Engine, TMUX_SESSION_OPTION};
use crate::shared::config::LayoutNode;

use super::execution::execute_layout_for_socket;
use super::{
    LayoutCommandsSpec, TmuxCommand, build_layout_commands, count_agent_panes, is_engine_command,
};

/// Inputs for opening the configured layout around a resumed agent.
pub struct ResumeLayoutSpec<'a> {
    pub session: &'a str,
    pub agent_session_id: &'a str,
    pub cwd: &'a str,
    pub window_name: &'a str,
    pub layout: &'a LayoutNode,
    pub engine: Engine,
    pub agent_command: &'a str,
    pub socket_path: Option<&'a str>,
}

trait LayoutExecutor {
    fn execute(
        &self,
        commands: &[TmuxCommand],
        session: &str,
        window_name: &str,
        socket_path: Option<&str>,
    ) -> anyhow::Result<String>;
    fn output(&self, socket_path: Option<&str>, args: &[&str]) -> anyhow::Result<String>;
}

struct TmuxLayoutExecutor;

impl LayoutExecutor for TmuxLayoutExecutor {
    fn execute(
        &self,
        commands: &[TmuxCommand],
        session: &str,
        window_name: &str,
        socket_path: Option<&str>,
    ) -> anyhow::Result<String> {
        execute_layout_for_socket(commands, session, window_name, true, socket_path)
            .map_err(Into::into)
    }

    fn output(&self, socket_path: Option<&str>, args: &[&str]) -> anyhow::Result<String> {
        crate::infra::tmux::run_tmux_output_for_socket(socket_path, args).map_err(Into::into)
    }
}

/// Opens the configured window and returns the resumed agent's pane ID when tmux reports it.
pub fn build_resume_layout(spec: ResumeLayoutSpec<'_>) -> anyhow::Result<Option<String>> {
    build_resume_layout_with_executor(spec, &TmuxLayoutExecutor)
}

fn build_resume_layout_with_executor(
    spec: ResumeLayoutSpec<'_>,
    executor: &impl LayoutExecutor,
) -> anyhow::Result<Option<String>> {
    let ResumeLayoutSpec {
        session,
        agent_session_id,
        cwd,
        window_name,
        layout,
        engine,
        agent_command,
        socket_path,
    } = spec;
    // A valid editor-only layout still needs a pane for the resumed agent.
    let layout_with_agent = if count_agent_panes(layout, engine) == 0 {
        LayoutNode::Split(crate::shared::config::SplitConfig {
            direction: crate::shared::config::SplitDirection::Horizontal,
            first: Box::new(layout.clone()),
            second: Box::new(LayoutNode::Pane(crate::shared::config::PaneConfig {
                command: engine.process_name().to_string(),
                focus: true,
            })),
        })
    } else {
        layout.clone()
    };
    let mut commands = build_layout_commands(LayoutCommandsSpec {
        session,
        cwd,
        window_name,
        layout: &layout_with_agent,
        model: None,
        reasoning_effort: None,
        prompt_file: None,
        engine,
        env_vars: &[],
        background: true,
        restore_automatic_rename: false,
    });
    let agent_command_slot = commands.iter().enumerate().find_map(|(index, command)| {
        if command.args.first().map(String::as_str) != Some("send-keys") {
            return None;
        }

        let command_index = command.args.iter().position(|arg| arg == "--")? + 1;
        let pane_command = command.args.get(command_index)?;
        if !is_engine_command(pane_command, engine) {
            return None;
        }

        let target_index = command.args.iter().position(|arg| arg == "-t")? + 1;
        let pane_target = command.args.get(target_index)?.clone();
        Some((index, command_index, pane_target))
    });
    let Some((agent_command_index, command_index, pane_target)) = agent_command_slot else {
        anyhow::bail!("tmux layout has no agent pane for {engine:?}");
    };
    commands[agent_command_index].args[command_index] = agent_command.to_string();
    commands.insert(
        agent_command_index,
        TmuxCommand {
            args: vec![
                "set-option".to_string(),
                "-p".to_string(),
                "-t".to_string(),
                pane_target,
                TMUX_SESSION_OPTION.to_string(),
                agent_session_id.to_string(),
            ],
        },
    );

    let window_id = executor.execute(&commands, session, window_name, socket_path)?;
    let format = format!("#{{pane_id}}\t#{{{TMUX_SESSION_OPTION}}}");
    let output = match executor.output(
        socket_path,
        &["list-panes", "-t", &window_id, "-F", &format],
    ) {
        Ok(output) => output,
        Err(error) => {
            tracing::warn!(%error, %window_id, "failed to resolve resumed agent pane");
            return Ok(None);
        }
    };
    let pane_id = find_agent_pane_id(&output, agent_session_id);
    if pane_id.is_none() {
        tracing::warn!(%window_id, "tmux layout did not report the resumed agent pane");
    }
    Ok(pane_id)
}

fn find_agent_pane_id(output: &str, agent_session_id: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let (pane_id, session_id) = line.split_once('\t')?;
        (session_id == agent_session_id).then(|| pane_id.to_string())
    })
}

#[cfg(test)]
mod pane_id_tests {
    use super::find_agent_pane_id;

    #[test]
    fn resume_resolves_the_agent_pane_from_its_session_marker() {
        let output = indoc::indoc! {"
            %17\tother-session
            %18\tfixture-session
        "};
        assert_eq!(
            find_agent_pane_id(output, "fixture-session"),
            Some("%18".to_string()),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use anyhow::Result;
    use serde_json::{Value, json};

    use super::{LayoutExecutor, ResumeLayoutSpec, build_resume_layout_with_executor};
    use crate::commands::agent::types::Engine;
    use crate::infra::tmux::layout::TmuxCommand;
    use crate::shared::config::{LayoutNode, PaneConfig, SplitConfig, SplitDirection};

    #[derive(Default)]
    struct FakeLayoutExecutor {
        calls: Mutex<Vec<Value>>,
    }

    impl LayoutExecutor for FakeLayoutExecutor {
        fn execute(
            &self,
            commands: &[TmuxCommand],
            session: &str,
            window_name: &str,
            socket_path: Option<&str>,
        ) -> Result<String> {
            self.calls.lock().unwrap().push(json!({
                "kind": "layout",
                "session": session,
                "windowName": window_name,
                "socketPath": socket_path,
                "commands": commands.iter().map(|command| command.args.clone()).collect::<Vec<_>>(),
            }));
            Ok("@fixture-window".to_string())
        }

        fn output(&self, socket_path: Option<&str>, args: &[&str]) -> Result<String> {
            self.calls.lock().unwrap().push(json!({
                "kind": "output",
                "socketPath": socket_path,
                "args": args,
            }));
            Ok("%18\tfixture-session\n".to_string())
        }
    }

    fn configured_layout() -> LayoutNode {
        LayoutNode::Split(SplitConfig {
            direction: SplitDirection::Horizontal,
            first: Box::new(LayoutNode::Pane(PaneConfig {
                command: "nvim".to_string(),
                focus: true,
            })),
            second: Box::new(LayoutNode::Pane(PaneConfig {
                command: "claude".to_string(),
                focus: false,
            })),
        })
    }

    #[test]
    fn resume_uses_the_configured_layout_and_runs_in_the_agent_pane() {
        let executor = FakeLayoutExecutor::default();
        let layout = configured_layout();

        let result = build_resume_layout_with_executor(
            ResumeLayoutSpec {
                session: "fixture-session",
                agent_session_id: "fixture-session",
                cwd: "/fixture/worktree",
                window_name: "fixture-agent",
                layout: &layout,
                engine: Engine::Codex,
                agent_command: "a agent resume fixture-session --engine codex",
                socket_path: Some("/fixture/tmux.sock"),
            },
            &executor,
        );

        assert_eq!(
            (result.unwrap(), executor.calls.lock().unwrap().clone()),
            (
                Some("%18".to_string()),
                vec![
                    json!({
                        "kind": "layout",
                        "session": "fixture-session",
                        "windowName": "fixture-agent",
                        "socketPath": "/fixture/tmux.sock",
                        "commands": [
                            ["new-window", "-d", "-t", "fixture-session", "-c", "/fixture/worktree", "-n", "fixture-agent"],
                            ["split-window", "-h", "-t", "fixture-session:=fixture-agent.1", "-c", "/fixture/worktree"],
                            ["select-pane", "-t", "fixture-session:=fixture-agent.1"],
                            ["send-keys", "-t", "fixture-session:=fixture-agent.1", "-l", "--", "nvim"],
                            ["send-keys", "-t", "fixture-session:=fixture-agent.1", "C-m"],
                            ["select-pane", "-t", "fixture-session:=fixture-agent.2"],
                            ["set-option", "-p", "-t", "fixture-session:=fixture-agent.2", "@armyknife-last-agent-session-id", "fixture-session"],
                            ["send-keys", "-t", "fixture-session:=fixture-agent.2", "-l", "--", "a agent resume fixture-session --engine codex"],
                            ["send-keys", "-t", "fixture-session:=fixture-agent.2", "C-m"],
                            ["select-pane", "-t", "fixture-session:=fixture-agent.1"],
                        ],
                    }),
                    json!({
                        "kind": "output",
                        "socketPath": "/fixture/tmux.sock",
                        "args": [
                            "list-panes",
                            "-t",
                            "@fixture-window",
                            "-F",
                            "#{pane_id}\t#{@armyknife-last-agent-session-id}",
                        ],
                    })
                ],
            ),
        );
    }

    #[test]
    fn resume_adds_an_agent_pane_when_the_layout_has_none() {
        let executor = FakeLayoutExecutor::default();
        let layout = LayoutNode::Pane(PaneConfig {
            command: "nvim".to_string(),
            focus: true,
        });

        let result = build_resume_layout_with_executor(
            ResumeLayoutSpec {
                session: "fixture-session",
                agent_session_id: "fixture-session",
                cwd: "/fixture/worktree",
                window_name: "fixture-agent",
                layout: &layout,
                engine: Engine::Codex,
                agent_command: "a agent resume fixture-session --engine codex",
                socket_path: Some("/fixture/tmux.sock"),
            },
            &executor,
        );

        assert_eq!(
            (result.unwrap(), executor.calls.lock().unwrap().clone()),
            (
                Some("%18".to_string()),
                vec![
                    json!({
                        "kind": "layout",
                        "session": "fixture-session",
                        "windowName": "fixture-agent",
                        "socketPath": "/fixture/tmux.sock",
                        "commands": [
                            ["new-window", "-d", "-t", "fixture-session", "-c", "/fixture/worktree", "-n", "fixture-agent"],
                            ["split-window", "-h", "-t", "fixture-session:=fixture-agent.1", "-c", "/fixture/worktree"],
                            ["select-pane", "-t", "fixture-session:=fixture-agent.1"],
                            ["send-keys", "-t", "fixture-session:=fixture-agent.1", "-l", "--", "nvim"],
                            ["send-keys", "-t", "fixture-session:=fixture-agent.1", "C-m"],
                            ["select-pane", "-t", "fixture-session:=fixture-agent.2"],
                            ["set-option", "-p", "-t", "fixture-session:=fixture-agent.2", "@armyknife-last-agent-session-id", "fixture-session"],
                            ["send-keys", "-t", "fixture-session:=fixture-agent.2", "-l", "--", "a agent resume fixture-session --engine codex"],
                            ["send-keys", "-t", "fixture-session:=fixture-agent.2", "C-m"],
                            ["select-pane", "-t", "fixture-session:=fixture-agent.2"],
                        ],
                    }),
                    json!({
                        "kind": "output",
                        "socketPath": "/fixture/tmux.sock",
                        "args": [
                            "list-panes",
                            "-t",
                            "@fixture-window",
                            "-F",
                            "#{pane_id}\t#{@armyknife-last-agent-session-id}",
                        ],
                    })
                ],
            ),
        );
    }
}
