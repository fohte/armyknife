//! Layout builder: converts a LayoutNode tree into tmux command sequences.

use crate::shared::config::{LayoutNode, SplitDirection};

/// A single tmux command represented as a list of arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxCommand {
    pub args: Vec<String>,
}

impl TmuxCommand {
    fn new(args: &[&str]) -> Self {
        Self {
            args: args.iter().map(|s| s.to_string()).collect(),
        }
    }
}

/// Information about a pane collected during tree traversal.
struct PaneEntry {
    command: String,
    focus: bool,
}

/// Build tmux command sequence from a LayoutNode tree.
///
/// Returns a list of tmux commands to create the window and configure panes.
/// The first command creates a new window, subsequent commands split panes.
pub fn build_layout_commands(
    session: &str,
    cwd: &str,
    window_name: &str,
    layout: &LayoutNode,
    prompt: Option<&str>,
) -> Vec<TmuxCommand> {
    let mut commands = Vec::new();
    let mut pane_entries: Vec<PaneEntry> = Vec::new();

    // Create new window (first pane is created automatically)
    commands.push(TmuxCommand::new(&[
        "new-window",
        "-t",
        session,
        "-c",
        cwd,
        "-n",
        window_name,
    ]));

    // Recursively process layout tree to collect split commands and pane info
    collect_layout(layout, &mut commands, &mut pane_entries, cwd);

    // Send commands to each pane
    for (i, entry) in pane_entries.iter().enumerate() {
        let pane_target = format!("{}", i + 1);
        let cmd = apply_prompt_if_claude(&entry.command, prompt);
        commands.push(TmuxCommand::new(&["select-pane", "-t", &pane_target]));
        commands.push(TmuxCommand::new(&["send-keys", &cmd, "C-m"]));
    }

    // Focus the last pane with focus: true
    let focus_pane_index = pane_entries
        .iter()
        .enumerate()
        .rfind(|(_, e)| e.focus)
        .map(|(i, _)| i);

    if let Some(idx) = focus_pane_index {
        let pane_target = format!("{}", idx + 1);
        commands.push(TmuxCommand::new(&["select-pane", "-t", &pane_target]));
    }

    commands
}

/// Recursively collect split commands and pane entries from the layout tree.
fn collect_layout(
    node: &LayoutNode,
    commands: &mut Vec<TmuxCommand>,
    panes: &mut Vec<PaneEntry>,
    cwd: &str,
) {
    match node {
        LayoutNode::Pane(pane) => {
            panes.push(PaneEntry {
                command: pane.command.clone(),
                focus: pane.focus,
            });
        }
        LayoutNode::Split(split) => {
            // Process first child (uses current pane)
            collect_layout(&split.first, commands, panes, cwd);

            // Split to create second child's pane
            let direction_flag = match split.direction {
                SplitDirection::Horizontal => "-h",
                SplitDirection::Vertical => "-v",
            };
            commands.push(TmuxCommand::new(&[
                "split-window",
                direction_flag,
                "-c",
                cwd,
            ]));

            // Process second child
            collect_layout(&split.second, commands, panes, cwd);
        }
    }
}

/// If the command starts with "claude", append the prompt with shell-safe quoting.
fn apply_prompt_if_claude(command: &str, prompt: Option<&str>) -> String {
    match prompt {
        Some(p) if command.starts_with("claude") => {
            // Escape single quotes for POSIX shell: replace ' with '\''
            let escaped = p.replace('\'', "'\\''");
            format!("{} '{}'", command, escaped)
        }
        _ => command.to_string(),
    }
}

/// Build and execute tmux layout from a LayoutNode tree.
///
/// Creates a new tmux window and configures panes according to the layout.
/// If prompt is provided, it's appended to pane commands that start with "claude".
pub fn build_layout(
    session: &str,
    cwd: &str,
    window_name: &str,
    layout: &LayoutNode,
    prompt: Option<&str>,
) -> super::Result<()> {
    let commands = build_layout_commands(session, cwd, window_name, layout, prompt);
    execute_commands(&commands)
}

/// Execute a sequence of TmuxCommand by chaining them with ";".
fn execute_commands(commands: &[TmuxCommand]) -> super::Result<()> {
    let mut args: Vec<&str> = Vec::new();
    for (i, cmd) in commands.iter().enumerate() {
        if i > 0 {
            args.push(";");
        }
        for arg in &cmd.args {
            args.push(arg);
        }
    }
    super::run_tmux(&args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::config::{PaneConfig, SplitConfig, SplitDirection};
    use rstest::rstest;

    /// Helper: create a TmuxCommand from a slice of string slices.
    fn cmd(args: &[&str]) -> TmuxCommand {
        TmuxCommand::new(args)
    }

    // =========================================================================
    // apply_prompt_if_claude tests
    // =========================================================================

    #[rstest]
    #[case::claude_with_prompt("claude", Some("fix the bug"), "claude 'fix the bug'")]
    #[case::claude_code_with_prompt("claude code", Some("review"), "claude code 'review'")]
    #[case::claude_without_prompt("claude", None, "claude")]
    #[case::non_claude_with_prompt("nvim", Some("fix the bug"), "nvim")]
    #[case::non_claude_without_prompt("bash", None, "bash")]
    #[case::prompt_with_single_quote(
        "claude",
        Some("fix the 'bug'"),
        "claude 'fix the '\\''bug'\\'''"
    )]
    fn test_apply_prompt_if_claude(
        #[case] command: &str,
        #[case] prompt: Option<&str>,
        #[case] expected: &str,
    ) {
        assert_eq!(apply_prompt_if_claude(command, prompt), expected);
    }

    // =========================================================================
    // build_layout_commands: single pane (no split)
    // =========================================================================

    #[test]
    fn single_pane_layout() {
        let layout = LayoutNode::Pane(PaneConfig {
            command: "nvim".to_string(),
            focus: true,
        });

        let commands = build_layout_commands("sess", "/tmp", "editor", &layout, None);

        assert_eq!(
            commands,
            vec![
                cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "editor"]),
                cmd(&["select-pane", "-t", "1"]),
                cmd(&["send-keys", "nvim", "C-m"]),
                cmd(&["select-pane", "-t", "1"]),
            ]
        );
    }

    // =========================================================================
    // build_layout_commands: 2 pane horizontal split
    // =========================================================================

    #[test]
    fn two_pane_horizontal_split() {
        let layout = LayoutNode::Split(SplitConfig {
            direction: SplitDirection::Horizontal,
            first: Box::new(LayoutNode::Pane(PaneConfig {
                command: "nvim".to_string(),
                focus: true,
            })),
            second: Box::new(LayoutNode::Pane(PaneConfig {
                command: "claude".to_string(),
                focus: false,
            })),
        });

        let commands = build_layout_commands("sess", "/tmp", "dev", &layout, None);

        assert_eq!(
            commands,
            vec![
                cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"]),
                cmd(&["split-window", "-h", "-c", "/tmp"]),
                cmd(&["select-pane", "-t", "1"]),
                cmd(&["send-keys", "nvim", "C-m"]),
                cmd(&["select-pane", "-t", "2"]),
                cmd(&["send-keys", "claude", "C-m"]),
                cmd(&["select-pane", "-t", "1"]),
            ]
        );
    }

    // =========================================================================
    // build_layout_commands: 2 pane vertical split
    // =========================================================================

    #[test]
    fn two_pane_vertical_split() {
        let layout = LayoutNode::Split(SplitConfig {
            direction: SplitDirection::Vertical,
            first: Box::new(LayoutNode::Pane(PaneConfig {
                command: "top".to_string(),
                focus: false,
            })),
            second: Box::new(LayoutNode::Pane(PaneConfig {
                command: "bash".to_string(),
                focus: true,
            })),
        });

        let commands = build_layout_commands("sess", "/tmp", "monitor", &layout, None);

        assert_eq!(
            commands,
            vec![
                cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "monitor"]),
                cmd(&["split-window", "-v", "-c", "/tmp"]),
                cmd(&["select-pane", "-t", "1"]),
                cmd(&["send-keys", "top", "C-m"]),
                cmd(&["select-pane", "-t", "2"]),
                cmd(&["send-keys", "bash", "C-m"]),
                cmd(&["select-pane", "-t", "2"]),
            ]
        );
    }

    // =========================================================================
    // build_layout_commands: 3 pane nested layout
    // (left: nvim, right-top: claude, right-bottom: bash)
    // =========================================================================

    #[test]
    fn three_pane_nested_layout() {
        let layout = LayoutNode::Split(SplitConfig {
            direction: SplitDirection::Horizontal,
            first: Box::new(LayoutNode::Pane(PaneConfig {
                command: "nvim".to_string(),
                focus: true,
            })),
            second: Box::new(LayoutNode::Split(SplitConfig {
                direction: SplitDirection::Vertical,
                first: Box::new(LayoutNode::Pane(PaneConfig {
                    command: "claude".to_string(),
                    focus: false,
                })),
                second: Box::new(LayoutNode::Pane(PaneConfig {
                    command: "bash".to_string(),
                    focus: false,
                })),
            })),
        });

        let commands = build_layout_commands("sess", "/tmp", "dev", &layout, None);

        assert_eq!(
            commands,
            vec![
                cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"]),
                cmd(&["split-window", "-h", "-c", "/tmp"]),
                cmd(&["split-window", "-v", "-c", "/tmp"]),
                cmd(&["select-pane", "-t", "1"]),
                cmd(&["send-keys", "nvim", "C-m"]),
                cmd(&["select-pane", "-t", "2"]),
                cmd(&["send-keys", "claude", "C-m"]),
                cmd(&["select-pane", "-t", "3"]),
                cmd(&["send-keys", "bash", "C-m"]),
                cmd(&["select-pane", "-t", "1"]),
            ]
        );
    }

    // =========================================================================
    // build_layout_commands: prompt appended to claude commands
    // =========================================================================

    #[test]
    fn prompt_appended_to_claude_commands() {
        let layout = LayoutNode::Split(SplitConfig {
            direction: SplitDirection::Horizontal,
            first: Box::new(LayoutNode::Pane(PaneConfig {
                command: "nvim".to_string(),
                focus: true,
            })),
            second: Box::new(LayoutNode::Pane(PaneConfig {
                command: "claude".to_string(),
                focus: false,
            })),
        });

        let commands = build_layout_commands("sess", "/tmp", "dev", &layout, Some("fix the bug"));

        assert_eq!(
            commands,
            vec![
                cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"]),
                cmd(&["split-window", "-h", "-c", "/tmp"]),
                cmd(&["select-pane", "-t", "1"]),
                cmd(&["send-keys", "nvim", "C-m"]),
                cmd(&["select-pane", "-t", "2"]),
                cmd(&["send-keys", "claude 'fix the bug'", "C-m"]),
                cmd(&["select-pane", "-t", "1"]),
            ]
        );
    }

    // =========================================================================
    // build_layout_commands: focus control
    // =========================================================================

    #[test]
    fn focus_last_focused_pane_wins() {
        // When multiple panes have focus: true, the last one wins
        let layout = LayoutNode::Split(SplitConfig {
            direction: SplitDirection::Horizontal,
            first: Box::new(LayoutNode::Pane(PaneConfig {
                command: "nvim".to_string(),
                focus: true,
            })),
            second: Box::new(LayoutNode::Pane(PaneConfig {
                command: "claude".to_string(),
                focus: true,
            })),
        });

        let commands = build_layout_commands("sess", "/tmp", "dev", &layout, None);

        // The last select-pane should target pane 2 (the last focused pane)
        let last_cmd = commands.last().unwrap();
        assert_eq!(last_cmd, &cmd(&["select-pane", "-t", "2"]));
    }

    #[test]
    fn no_focus_pane_omits_final_select() {
        // When no pane has focus: true, no final select-pane is emitted
        let layout = LayoutNode::Split(SplitConfig {
            direction: SplitDirection::Horizontal,
            first: Box::new(LayoutNode::Pane(PaneConfig {
                command: "nvim".to_string(),
                focus: false,
            })),
            second: Box::new(LayoutNode::Pane(PaneConfig {
                command: "bash".to_string(),
                focus: false,
            })),
        });

        let commands = build_layout_commands("sess", "/tmp", "dev", &layout, None);

        // Last command should be a send-keys, not select-pane for focus
        let last_cmd = commands.last().unwrap();
        assert_eq!(last_cmd, &cmd(&["send-keys", "bash", "C-m"]));
    }
}
