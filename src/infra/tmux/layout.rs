//! Layout builder: converts a LayoutNode tree into tmux command sequences.

use std::path::Path;

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
/// If `prompt_file` is provided, claude pane commands will read the prompt
/// from the file at shell execution time and delete it afterward.
/// `env_vars` are set as tmux session-level environment variables so that
/// all panes in the window inherit them.
pub fn build_layout_commands(
    session: &str,
    cwd: &str,
    window_name: &str,
    layout: &LayoutNode,
    prompt_file: Option<&Path>,
    env_vars: &[(&str, &str)],
) -> Vec<TmuxCommand> {
    let mut commands = Vec::new();
    let mut pane_entries: Vec<PaneEntry> = Vec::new();

    // Set session-level environment variables before creating the window so
    // all spawned panes inherit them via tmux's update-environment mechanism.
    for (key, value) in env_vars {
        commands.push(TmuxCommand::new(&[
            "set-environment",
            "-t",
            session,
            key,
            value,
        ]));
    }

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

    // Recursively process layout tree to collect split commands and pane info.
    // Pane indices are 1-based in tmux (new-window creates pane 1).
    collect_layout(layout, &mut commands, &mut pane_entries, cwd, 1);

    // Find the last claude pane index so only it performs temp file cleanup
    let last_claude_index = prompt_file.and_then(|_| {
        pane_entries
            .iter()
            .rposition(|e| e.command.starts_with("claude"))
    });

    // Send commands to each pane
    for (i, entry) in pane_entries.iter().enumerate() {
        let pane_target = format!("{}", i + 1);
        let cleanup = last_claude_index == Some(i);
        let cmd = apply_prompt_if_claude(&entry.command, prompt_file, cleanup);
        commands.push(TmuxCommand::new(&["select-pane", "-t", &pane_target]));
        // Use -l to send the command literally (prevents interpreting special key sequences),
        // then send Enter separately.
        commands.push(TmuxCommand::new(&["send-keys", "-l", "--", &cmd]));
        commands.push(TmuxCommand::new(&["send-keys", "C-m"]));
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

    // Unset session-level env vars after all panes have been created to
    // prevent leaking into subsequent windows in the same tmux session.
    for (key, _) in env_vars {
        commands.push(TmuxCommand::new(&[
            "set-environment",
            "-u",
            "-t",
            session,
            key,
        ]));
    }

    commands
}

/// Recursively collect split commands and pane entries from the layout tree.
///
/// `pane_offset` is the 1-based index of the first pane that will be created
/// for this subtree. It is used to emit `-t` targeting on `split-window`
/// commands so that nested splits target the correct pane regardless of which
/// pane is currently focused.
fn collect_layout(
    node: &LayoutNode,
    commands: &mut Vec<TmuxCommand>,
    panes: &mut Vec<PaneEntry>,
    cwd: &str,
    pane_offset: usize,
) {
    match node {
        LayoutNode::Pane(pane) => {
            panes.push(PaneEntry {
                command: pane.command.clone(),
                focus: pane.focus,
            });
        }
        LayoutNode::Split(split) => {
            // Process first child (uses current pane at pane_offset)
            collect_layout(&split.first, commands, panes, cwd, pane_offset);

            let first_pane_count = count_panes(&split.first);

            // Split the first child's root pane to create second child's pane.
            // Use -t to explicitly target the pane, because recursing into
            // first may have created additional panes that changed the active pane.
            let direction_flag = match split.direction {
                SplitDirection::Horizontal => "-h",
                SplitDirection::Vertical => "-v",
            };
            let target = format!("{}", pane_offset);
            commands.push(TmuxCommand::new(&[
                "split-window",
                direction_flag,
                "-t",
                &target,
                "-c",
                cwd,
            ]));

            // Process second child
            let second_offset = pane_offset + first_pane_count;
            collect_layout(&split.second, commands, panes, cwd, second_offset);
        }
    }
}

/// Count the total number of leaf panes in a layout subtree.
fn count_panes(node: &LayoutNode) -> usize {
    match node {
        LayoutNode::Pane(_) => 1,
        LayoutNode::Split(split) => count_panes(&split.first) + count_panes(&split.second),
    }
}

/// If the command starts with "claude", append the prompt file path.
///
/// Uses `$(cat <path>)` to read the prompt at shell execution time.
/// If `cleanup` is true, also deletes the temp file after reading.
/// Only the last claude pane should set `cleanup = true` to avoid
/// deleting the file before other panes have read it.
fn apply_prompt_if_claude(command: &str, prompt_file: Option<&Path>, cleanup: bool) -> String {
    match prompt_file {
        Some(path) => {
            if !command.starts_with("claude") {
                return command.to_string();
            }
            let path_str = path.display().to_string();
            let escaped_path = shlex::try_quote(&path_str)
                .map(|c| c.into_owned())
                .unwrap_or(path_str);
            if cleanup {
                format!("{command} \"$(cat {escaped_path})\" ; rm {escaped_path}")
            } else {
                format!("{command} \"$(cat {escaped_path})\"")
            }
        }
        _ => command.to_string(),
    }
}

/// Build and execute tmux layout from a LayoutNode tree.
///
/// Creates a new tmux window and configures panes according to the layout.
/// If prompt is provided, writes it to a temp file and passes the path to
/// claude pane commands. The temp file is read and deleted by the shell
/// command at execution time.
/// `env_vars` are forwarded to `build_layout_commands` as tmux session-level
/// environment variables.
pub fn build_layout(
    session: &str,
    cwd: &str,
    window_name: &str,
    layout: &LayoutNode,
    prompt: Option<&str>,
    env_vars: &[(&str, &str)],
) -> anyhow::Result<()> {
    let prompt_file = prompt.map(write_prompt_file).transpose()?;
    let prompt_path = prompt_file.as_deref();
    let commands = build_layout_commands(session, cwd, window_name, layout, prompt_path, env_vars);
    execute_commands(&commands)?;
    Ok(())
}

/// Write prompt to a temp file that persists until the shell command reads it.
fn write_prompt_file(prompt: &str) -> anyhow::Result<std::path::PathBuf> {
    use anyhow::Context;

    let prompt_file = tempfile::Builder::new()
        .prefix("claude-prompt-")
        .suffix(".txt")
        .tempfile()
        .context("Failed to create temp file for prompt")?;

    std::fs::write(prompt_file.path(), prompt).context("Failed to write prompt to temp file")?;

    // Keep the temp file so it persists after this function returns.
    // The shell command will delete it after reading.
    prompt_file
        .into_temp_path()
        .keep()
        .context("Failed to persist prompt temp file")
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
    use crate::shared::env_var::EnvVars;
    use rstest::rstest;
    use std::path::PathBuf;

    /// Helper: create a TmuxCommand from a slice of string slices.
    fn cmd(args: &[&str]) -> TmuxCommand {
        TmuxCommand::new(args)
    }

    // =========================================================================
    // apply_prompt_if_claude tests
    // =========================================================================

    #[rstest]
    #[case::claude_without_prompt("claude", None)]
    #[case::non_claude_with_prompt("nvim", Some("/tmp/prompt.txt"))]
    #[case::non_claude_without_prompt("bash", None)]
    fn test_apply_prompt_if_claude_no_expansion(#[case] command: &str, #[case] path: Option<&str>) {
        let path_buf = path.map(PathBuf::from);
        let result = apply_prompt_if_claude(command, path_buf.as_deref(), true);
        assert_eq!(result, command);
    }

    #[rstest]
    #[case::claude_with_cleanup(
        "claude",
        "/tmp/prompt.txt",
        true,
        "claude \"$(cat /tmp/prompt.txt)\" ; rm /tmp/prompt.txt"
    )]
    #[case::claude_without_cleanup(
        "claude",
        "/tmp/prompt.txt",
        false,
        "claude \"$(cat /tmp/prompt.txt)\""
    )]
    #[case::claude_code_with_cleanup(
        "claude code",
        "/tmp/prompt.txt",
        true,
        "claude code \"$(cat /tmp/prompt.txt)\" ; rm /tmp/prompt.txt"
    )]
    fn test_apply_prompt_if_claude_with_file(
        #[case] command: &str,
        #[case] path: &str,
        #[case] cleanup: bool,
        #[case] expected: &str,
    ) {
        let path_buf = PathBuf::from(path);
        let result = apply_prompt_if_claude(command, Some(&path_buf), cleanup);
        assert_eq!(result, expected);
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

        let commands = build_layout_commands("sess", "/tmp", "editor", &layout, None, &[]);

        assert_eq!(
            commands,
            vec![
                cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "editor"]),
                cmd(&["select-pane", "-t", "1"]),
                cmd(&["send-keys", "-l", "--", "nvim"]),
                cmd(&["send-keys", "C-m"]),
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

        let commands = build_layout_commands("sess", "/tmp", "dev", &layout, None, &[]);

        assert_eq!(
            commands,
            vec![
                cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"]),
                cmd(&["split-window", "-h", "-t", "1", "-c", "/tmp"]),
                cmd(&["select-pane", "-t", "1"]),
                cmd(&["send-keys", "-l", "--", "nvim"]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "2"]),
                cmd(&["send-keys", "-l", "--", "claude"]),
                cmd(&["send-keys", "C-m"]),
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

        let commands = build_layout_commands("sess", "/tmp", "monitor", &layout, None, &[]);

        assert_eq!(
            commands,
            vec![
                cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "monitor"]),
                cmd(&["split-window", "-v", "-t", "1", "-c", "/tmp"]),
                cmd(&["select-pane", "-t", "1"]),
                cmd(&["send-keys", "-l", "--", "top"]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "2"]),
                cmd(&["send-keys", "-l", "--", "bash"]),
                cmd(&["send-keys", "C-m"]),
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

        let commands = build_layout_commands("sess", "/tmp", "dev", &layout, None, &[]);

        assert_eq!(
            commands,
            vec![
                cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"]),
                cmd(&["split-window", "-h", "-t", "1", "-c", "/tmp"]),
                cmd(&["split-window", "-v", "-t", "2", "-c", "/tmp"]),
                cmd(&["select-pane", "-t", "1"]),
                cmd(&["send-keys", "-l", "--", "nvim"]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "2"]),
                cmd(&["send-keys", "-l", "--", "claude"]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "3"]),
                cmd(&["send-keys", "-l", "--", "bash"]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "1"]),
            ]
        );
    }

    // =========================================================================
    // build_layout_commands: prompt file appended to claude commands
    // =========================================================================

    #[test]
    fn prompt_file_appended_to_claude_commands() {
        let prompt_path = PathBuf::from("/tmp/claude-prompt-test.txt");
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

        let commands =
            build_layout_commands("sess", "/tmp", "dev", &layout, Some(&prompt_path), &[]);

        assert_eq!(
            commands,
            vec![
                cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"]),
                cmd(&["split-window", "-h", "-t", "1", "-c", "/tmp"]),
                cmd(&["select-pane", "-t", "1"]),
                cmd(&["send-keys", "-l", "--", "nvim"]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "2"]),
                cmd(&[
                    "send-keys",
                    "-l",
                    "--",
                    "claude \"$(cat /tmp/claude-prompt-test.txt)\" ; rm /tmp/claude-prompt-test.txt",
                ]),
                cmd(&["send-keys", "C-m"]),
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

        let commands = build_layout_commands("sess", "/tmp", "dev", &layout, None, &[]);

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

        let commands = build_layout_commands("sess", "/tmp", "dev", &layout, None, &[]);

        // Last command should be a send-keys C-m, not select-pane for focus
        let last_cmd = commands.last().unwrap();
        assert_eq!(last_cmd, &cmd(&["send-keys", "C-m"]));
    }

    // =========================================================================
    // build_layout_commands: left-nested layout (first child is a split)
    // Regression test for: split-window targeting wrong pane when first child
    // is itself a Split node.
    // =========================================================================

    #[test]
    fn left_nested_layout_targets_correct_pane() {
        // Expected: left-top=a, left-bottom=b, right=c (full height)
        let layout = LayoutNode::Split(SplitConfig {
            direction: SplitDirection::Horizontal,
            first: Box::new(LayoutNode::Split(SplitConfig {
                direction: SplitDirection::Vertical,
                first: Box::new(LayoutNode::Pane(PaneConfig {
                    command: "a".to_string(),
                    focus: true,
                })),
                second: Box::new(LayoutNode::Pane(PaneConfig {
                    command: "b".to_string(),
                    focus: false,
                })),
            })),
            second: Box::new(LayoutNode::Pane(PaneConfig {
                command: "c".to_string(),
                focus: false,
            })),
        });

        let commands = build_layout_commands("sess", "/tmp", "dev", &layout, None, &[]);

        assert_eq!(
            commands,
            vec![
                cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"]),
                // Inner vertical split: splits pane 1 vertically -> panes 1, 2
                cmd(&["split-window", "-v", "-t", "1", "-c", "/tmp"]),
                // Outer horizontal split: splits pane 1 (the root of first subtree)
                // horizontally -> creates pane 3 for "c"
                cmd(&["split-window", "-h", "-t", "1", "-c", "/tmp"]),
                cmd(&["select-pane", "-t", "1"]),
                cmd(&["send-keys", "-l", "--", "a"]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "2"]),
                cmd(&["send-keys", "-l", "--", "b"]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "3"]),
                cmd(&["send-keys", "-l", "--", "c"]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "1"]),
            ]
        );
    }

    // =========================================================================
    // build_layout_commands: multiple claude panes with prompt file
    // Only the last claude pane should delete the temp file.
    // =========================================================================

    #[test]
    fn multiple_claude_panes_only_last_deletes_prompt_file() {
        let prompt_path = PathBuf::from("/tmp/prompt.txt");
        let layout = LayoutNode::Split(SplitConfig {
            direction: SplitDirection::Horizontal,
            first: Box::new(LayoutNode::Pane(PaneConfig {
                command: "claude -p agent1".to_string(),
                focus: true,
            })),
            second: Box::new(LayoutNode::Pane(PaneConfig {
                command: "claude -p agent2".to_string(),
                focus: false,
            })),
        });

        let commands =
            build_layout_commands("sess", "/tmp", "dev", &layout, Some(&prompt_path), &[]);

        assert_eq!(
            commands,
            vec![
                cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"]),
                cmd(&["split-window", "-h", "-t", "1", "-c", "/tmp"]),
                cmd(&["select-pane", "-t", "1"]),
                // First claude pane: reads prompt but does NOT delete file
                cmd(&[
                    "send-keys",
                    "-l",
                    "--",
                    "claude -p agent1 \"$(cat /tmp/prompt.txt)\"",
                ]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "2"]),
                // Last claude pane: reads prompt AND deletes file
                cmd(&[
                    "send-keys",
                    "-l",
                    "--",
                    "claude -p agent2 \"$(cat /tmp/prompt.txt)\" ; rm /tmp/prompt.txt",
                ]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "1"]),
            ]
        );
    }

    // =========================================================================
    // build_layout_commands: env_vars inject set-environment before new-window
    // =========================================================================

    #[test]
    fn env_vars_prepended_as_set_environment() {
        let layout = LayoutNode::Pane(PaneConfig {
            command: "claude".to_string(),
            focus: true,
        });

        let label_key = EnvVars::session_label_name();
        let ancestors_key = EnvVars::ancestor_session_ids_name();
        let env_vars = [
            (label_key, "my-label"),
            (ancestors_key, "parent-1,parent-2"),
        ];
        let commands = build_layout_commands("sess", "/tmp", "dev", &layout, None, &env_vars);

        // set-environment commands should come before new-window
        assert_eq!(
            commands[0],
            cmd(&["set-environment", "-t", "sess", label_key, "my-label"])
        );
        assert_eq!(
            commands[1],
            cmd(&[
                "set-environment",
                "-t",
                "sess",
                ancestors_key,
                "parent-1,parent-2",
            ])
        );
        assert_eq!(
            commands[2],
            cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"])
        );

        // Unset commands should come at the end to prevent leaking
        let n = commands.len();
        assert_eq!(
            commands[n - 2],
            cmd(&["set-environment", "-u", "-t", "sess", label_key])
        );
        assert_eq!(
            commands[n - 1],
            cmd(&["set-environment", "-u", "-t", "sess", ancestors_key])
        );
    }

    #[test]
    fn empty_env_vars_no_set_environment() {
        let layout = LayoutNode::Pane(PaneConfig {
            command: "claude".to_string(),
            focus: true,
        });

        let commands = build_layout_commands("sess", "/tmp", "dev", &layout, None, &[]);

        // First command should be new-window, not set-environment
        assert_eq!(
            commands[0],
            cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"])
        );
    }
}
