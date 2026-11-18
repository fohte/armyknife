use crate::commands::agent::types::Engine;
use crate::shared::config::LayoutNode;

use super::execution::execute_layout_for_socket;
use super::{LayoutCommandsSpec, TmuxCommand, build_layout_commands, is_engine_command};

/// Inputs for opening the configured layout around a resumed agent.
pub struct ResumeLayoutSpec<'a> {
    pub session: &'a str,
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
}

/// Opens a new window with the configured panes and resumes the agent in its pane.
pub fn build_resume_layout(spec: ResumeLayoutSpec<'_>) -> anyhow::Result<String> {
    build_resume_layout_with_executor(spec, &TmuxLayoutExecutor)
}

fn build_resume_layout_with_executor(
    spec: ResumeLayoutSpec<'_>,
    executor: &impl LayoutExecutor,
) -> anyhow::Result<String> {
    let ResumeLayoutSpec {
        session,
        cwd,
        window_name,
        layout,
        engine,
        agent_command,
        socket_path,
    } = spec;
    let mut commands = build_layout_commands(LayoutCommandsSpec {
        session,
        cwd,
        window_name,
        layout,
        model: None,
        reasoning_effort: None,
        prompt_file: None,
        engine,
        env_vars: &[],
        background: true,
        restore_automatic_rename: false,
    });
    let agent_command_slot = commands.iter_mut().find_map(|command| {
        if command.args.first().map(String::as_str) != Some("send-keys") {
            return None;
        }

        let command_index = command.args.iter().position(|arg| arg == "--")? + 1;
        let pane_command = command.args.get(command_index)?;
        is_engine_command(pane_command, engine).then_some((command, command_index))
    });
    let Some((command, command_index)) = agent_command_slot else {
        anyhow::bail!("tmux layout has no agent pane for {engine:?}");
    };
    command.args[command_index] = agent_command.to_string();

    executor.execute(&commands, session, window_name, socket_path)
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
                "session": session,
                "windowName": window_name,
                "socketPath": socket_path,
                "commands": commands.iter().map(|command| command.args.clone()).collect::<Vec<_>>(),
            }));
            Ok("fixture-window-id".to_string())
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
                "fixture-window-id".to_string(),
                vec![json!({
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
                        ["send-keys", "-t", "fixture-session:=fixture-agent.2", "-l", "--", "a agent resume fixture-session --engine codex"],
                        ["send-keys", "-t", "fixture-session:=fixture-agent.2", "C-m"],
                        ["select-pane", "-t", "fixture-session:=fixture-agent.1"],
                    ],
                })],
            ),
        );
    }
}
