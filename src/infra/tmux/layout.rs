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

/// The pane-targeting prefix used to address panes in a background-created
/// window before its real window ID is known (see `execute_background_layout`).
fn background_pane_prefix(session: &str, window_name: &str) -> String {
    format!("{session}:={window_name}.")
}

/// Inputs for `build_layout_commands`, grouped to keep its argument count in
/// check.
pub struct LayoutCommandsSpec<'a> {
    pub session: &'a str,
    pub cwd: &'a str,
    pub window_name: &'a str,
    pub layout: &'a LayoutNode,
    /// Inserted right after `claude` in claude pane commands.
    pub model: Option<&'a str>,
    /// When set, claude pane commands read the prompt from this file at
    /// shell execution time and delete it afterward.
    pub prompt_file: Option<&'a Path>,
    /// Set as tmux session-level environment variables so all panes in the
    /// window inherit them.
    pub env_vars: &'a [(&'a str, &'a str)],
    pub background: bool,
    /// Turns `automatic-rename` back on right after window creation, undoing
    /// tmux's default of disabling it whenever a window is created with an
    /// explicit name (`-n`).
    pub restore_automatic_rename: bool,
}

/// Build tmux command sequence from a LayoutNode tree.
///
/// Returns a list of tmux commands to create the window and configure panes.
/// The first command creates a new window, subsequent commands split panes.
pub fn build_layout_commands(spec: LayoutCommandsSpec) -> Vec<TmuxCommand> {
    let LayoutCommandsSpec {
        session,
        cwd,
        window_name,
        layout,
        model,
        prompt_file,
        env_vars,
        background,
        restore_automatic_rename,
    } = spec;

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

    // In background mode, create the window detached (`-d`) and address every
    // subsequent pane operation by the fully-qualified `{session}:={name}.N`
    // target so the attached client's active window never flips, not even for
    // a single frame.
    let new_window_args = if background {
        vec![
            "new-window",
            "-d",
            "-t",
            session,
            "-c",
            cwd,
            "-n",
            window_name,
        ]
    } else {
        vec!["new-window", "-t", session, "-c", cwd, "-n", window_name]
    };
    commands.push(TmuxCommand::new(&new_window_args));

    // Placed right after `new-window`, before any pane targets get rewritten
    // to a captured window ID (see `execute_background_layout`), so this can
    // always address the window by its just-created, still-unrenamed name.
    if restore_automatic_rename {
        let window_target = format!("{session}:={window_name}");
        commands.push(TmuxCommand::new(&[
            "set-option",
            "-w",
            "-t",
            &window_target,
            "automatic-rename",
            "on",
        ]));
    }

    let pane_prefix = if background {
        background_pane_prefix(session, window_name)
    } else {
        String::new()
    };

    // Recursively process layout tree to collect split commands and pane info.
    // Pane indices are 1-based in tmux (new-window creates pane 1).
    collect_layout(
        layout,
        &mut commands,
        &mut pane_entries,
        cwd,
        1,
        &pane_prefix,
    );

    // Find the last claude pane index so only it performs temp file cleanup
    let last_claude_index = prompt_file.and_then(|_| {
        pane_entries
            .iter()
            .rposition(|e| e.command.starts_with("claude"))
    });

    // Send commands to each pane
    for (i, entry) in pane_entries.iter().enumerate() {
        let pane_target = format!("{pane_prefix}{}", i + 1);
        let cleanup = last_claude_index == Some(i);
        let cmd = apply_prompt_if_claude(&entry.command, model, prompt_file, cleanup);
        commands.push(TmuxCommand::new(&["select-pane", "-t", &pane_target]));
        // Use -l to send the command literally (prevents interpreting special key sequences),
        // then send Enter separately. In background mode the active pane stays
        // on the user's current window, so send-keys must target the new pane
        // explicitly; in foreground the prior select-pane already makes the
        // target active so the -t argument is omitted to keep behavior intact.
        if background {
            commands.push(TmuxCommand::new(&[
                "send-keys",
                "-t",
                &pane_target,
                "-l",
                "--",
                &cmd,
            ]));
            commands.push(TmuxCommand::new(&["send-keys", "-t", &pane_target, "C-m"]));
        } else {
            commands.push(TmuxCommand::new(&["send-keys", "-l", "--", &cmd]));
            commands.push(TmuxCommand::new(&["send-keys", "C-m"]));
        }
    }

    // Focus the last pane with focus: true
    let focus_pane_index = pane_entries
        .iter()
        .enumerate()
        .rfind(|(_, e)| e.focus)
        .map(|(i, _)| i);

    if let Some(idx) = focus_pane_index {
        let pane_target = format!("{pane_prefix}{}", idx + 1);
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
    pane_prefix: &str,
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
            collect_layout(&split.first, commands, panes, cwd, pane_offset, pane_prefix);

            let first_pane_count = count_panes(&split.first);

            // Split the first child's root pane to create second child's pane.
            // Use -t to explicitly target the pane, because recursing into
            // first may have created additional panes that changed the active pane.
            let direction_flag = match split.direction {
                SplitDirection::Horizontal => "-h",
                SplitDirection::Vertical => "-v",
            };
            let target = format!("{pane_prefix}{pane_offset}");
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
            collect_layout(
                &split.second,
                commands,
                panes,
                cwd,
                second_offset,
                pane_prefix,
            );
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

/// If the command starts with "claude", insert `--model <model>` right after
/// `claude` and append the prompt file path.
///
/// Uses `$(cat <path>)` to read the prompt at shell execution time.
/// If `cleanup` is true, also deletes the temp file after reading.
/// Only the last claude pane should set `cleanup = true` to avoid
/// deleting the file before other panes have read it.
fn apply_prompt_if_claude(
    command: &str,
    model: Option<&str>,
    prompt_file: Option<&Path>,
    cleanup: bool,
) -> String {
    if !command.starts_with("claude") {
        return command.to_string();
    }

    // Restrict to an exact "claude" command (optionally followed by a
    // space-separated rest) so a differently-named pane command that merely
    // starts with "claude" (e.g. a "claude-code" wrapper script) isn't
    // mangled by splicing --model into the middle of its name.
    let command = match model {
        Some(model) if command == "claude" || command.starts_with("claude ") => {
            let escaped_model = shlex::try_quote(model)
                .map(|c| c.into_owned())
                .unwrap_or_else(|_| model.to_string());
            format!(
                "claude --model {escaped_model}{}",
                &command["claude".len()..]
            )
        }
        _ => command.to_string(),
    };

    match prompt_file {
        Some(path) => {
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
        None => command,
    }
}

/// Inputs for `build_layout`, grouped to keep its argument count in check.
pub struct LayoutSpec<'a> {
    pub session: &'a str,
    pub cwd: &'a str,
    pub window_name: &'a str,
    pub layout: &'a LayoutNode,
    pub model: Option<&'a str>,
    /// Written to a temp file and passed to claude pane commands; the temp
    /// file is read and deleted by the shell command at execution time.
    pub prompt: Option<&'a str>,
    /// Set as tmux session-level environment variables so all panes in the
    /// window inherit them.
    pub env_vars: &'a [(&'a str, &'a str)],
    pub background: bool,
    /// Forwarded to `build_layout_commands` (see `LayoutCommandsSpec`).
    pub restore_automatic_rename: bool,
}

/// Build and execute tmux layout from a LayoutNode tree.
///
/// Creates a new tmux window and configures panes according to the layout.
pub fn build_layout(spec: LayoutSpec) -> anyhow::Result<()> {
    let LayoutSpec {
        session,
        cwd,
        window_name,
        layout,
        model,
        prompt,
        env_vars,
        background,
        restore_automatic_rename,
    } = spec;

    let prompt_file = prompt.map(write_prompt_file).transpose()?;
    let prompt_path = prompt_file.as_deref();
    let commands = build_layout_commands(LayoutCommandsSpec {
        session,
        cwd,
        window_name,
        layout,
        model,
        prompt_file: prompt_path,
        env_vars,
        background,
        restore_automatic_rename,
    });

    if background {
        execute_background_layout(&commands, session, window_name)?;
    } else {
        execute_commands(&commands)?;
    }

    Ok(())
}

/// Flattens a sequence of `TmuxCommand` into a single arg list joined by `;`,
/// the wire format tmux uses to chain multiple commands in one invocation.
fn flatten_commands(commands: &[TmuxCommand]) -> Vec<&str> {
    let mut args: Vec<&str> = Vec::new();
    for (i, cmd) in commands.iter().enumerate() {
        if i > 0 {
            args.push(";");
        }
        for arg in &cmd.args {
            args.push(arg);
        }
    }
    args
}

/// Executes a background-mode layout: runs `new-window` (plus any preceding
/// `set-environment` commands) on its own so the real window ID can be
/// captured via `-P -F "#{window_id}"`, then rewrites the remaining commands'
/// `{session}:={window_name}.` pane targets to `{window_id}.` before running
/// them.
///
/// This indirection exists because tmux's target parser splits on `.` to
/// separate window from pane, so a session-qualified target is ambiguous when
/// the window name itself contains a `.` (e.g. a branch name like
/// `copier-update/v0.8.13` becomes window name `copier-update-v0.8.13`).
/// Window IDs (e.g. `@42`) never contain `.`, so targeting by ID sidesteps the
/// ambiguity entirely.
fn execute_background_layout(
    commands: &[TmuxCommand],
    session: &str,
    window_name: &str,
) -> super::Result<()> {
    let new_window_idx = find_new_window_index(commands).ok_or_else(|| {
        super::TmuxError::Internal("new-window command not found in layout".to_string())
    })?;

    let setup = &commands[..=new_window_idx];
    let rest = &commands[new_window_idx + 1..];

    let mut setup_args = flatten_commands(setup);
    setup_args.extend(["-P", "-F", "#{window_id}"]);
    let window_id = super::run_tmux_output(&setup_args)?;

    let old_prefix = background_pane_prefix(session, window_name);
    let new_prefix = format!("{window_id}.");
    execute_commands(&rewrite_pane_targets(rest, &old_prefix, &new_prefix))
}

/// Finds the index of the `new-window` command in a layout's command list.
fn find_new_window_index(commands: &[TmuxCommand]) -> Option<usize> {
    commands
        .iter()
        .position(|cmd| cmd.args.first().map(String::as_str) == Some("new-window"))
}

/// Rewrites pane-targeting command args from `{old_prefix}{pane}` to
/// `{new_prefix}{pane}`.
fn rewrite_pane_targets(
    commands: &[TmuxCommand],
    old_prefix: &str,
    new_prefix: &str,
) -> Vec<TmuxCommand> {
    commands
        .iter()
        .map(|cmd| TmuxCommand {
            args: cmd
                .args
                .iter()
                .map(|arg| match arg.strip_prefix(old_prefix) {
                    Some(rest) => format!("{new_prefix}{rest}"),
                    None => arg.clone(),
                })
                .collect(),
        })
        .collect()
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
    super::run_tmux(&flatten_commands(commands))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::config::{PaneConfig, SplitConfig, SplitDirection};
    use crate::shared::env_var::EnvVars;
    use rstest::{fixture, rstest};
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
        let result = apply_prompt_if_claude(command, None, path_buf.as_deref(), true);
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
        let result = apply_prompt_if_claude(command, None, Some(&path_buf), cleanup);
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case::claude_with_model_no_prompt("claude", Some("opus"), None, "claude --model opus")]
    #[case::claude_with_extra_args_and_model(
        "claude -p agent1",
        Some("opus"),
        None,
        "claude --model opus -p agent1"
    )]
    #[case::claude_without_model_unchanged("claude", None, None, "claude")]
    #[case::non_claude_with_model_unchanged("nvim", Some("opus"), None, "nvim")]
    #[case::claude_prefixed_other_command_unchanged(
        "claude-code",
        Some("opus"),
        None,
        "claude-code"
    )]
    #[case::claude_with_model_and_prompt(
        "claude",
        Some("opus"),
        Some("/tmp/prompt.txt"),
        "claude --model opus \"$(cat /tmp/prompt.txt)\" ; rm /tmp/prompt.txt"
    )]
    fn test_apply_prompt_if_claude_with_model(
        #[case] command: &str,
        #[case] model: Option<&str>,
        #[case] path: Option<&str>,
        #[case] expected: &str,
    ) {
        let path_buf = path.map(PathBuf::from);
        let result = apply_prompt_if_claude(command, model, path_buf.as_deref(), true);
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

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "editor",
            layout: &layout,
            model: None,
            prompt_file: None,
            env_vars: &[],
            background: false,
            restore_automatic_rename: false,
        });

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

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: None,
            prompt_file: None,
            env_vars: &[],
            background: false,
            restore_automatic_rename: false,
        });

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

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "monitor",
            layout: &layout,
            model: None,
            prompt_file: None,
            env_vars: &[],
            background: false,
            restore_automatic_rename: false,
        });

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

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: None,
            prompt_file: None,
            env_vars: &[],
            background: false,
            restore_automatic_rename: false,
        });

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

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: None,
            prompt_file: Some(&prompt_path),
            env_vars: &[],
            background: false,
            restore_automatic_rename: false,
        });

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

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: None,
            prompt_file: None,
            env_vars: &[],
            background: false,
            restore_automatic_rename: false,
        });

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

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: None,
            prompt_file: None,
            env_vars: &[],
            background: false,
            restore_automatic_rename: false,
        });

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

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: None,
            prompt_file: None,
            env_vars: &[],
            background: false,
            restore_automatic_rename: false,
        });

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

    /// Two claude panes side by side, shared by tests that verify per-pane
    /// command shaping (prompt file cleanup, --model) applies consistently.
    #[fixture]
    fn two_claude_pane_layout() -> LayoutNode {
        LayoutNode::Split(SplitConfig {
            direction: SplitDirection::Horizontal,
            first: Box::new(LayoutNode::Pane(PaneConfig {
                command: "claude -p agent1".to_string(),
                focus: true,
            })),
            second: Box::new(LayoutNode::Pane(PaneConfig {
                command: "claude -p agent2".to_string(),
                focus: false,
            })),
        })
    }

    // =========================================================================
    // build_layout_commands: multiple claude panes with prompt file
    // Only the last claude pane should delete the temp file.
    // =========================================================================

    #[rstest]
    fn multiple_claude_panes_only_last_deletes_prompt_file(two_claude_pane_layout: LayoutNode) {
        let prompt_path = PathBuf::from("/tmp/prompt.txt");
        let layout = two_claude_pane_layout;

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: None,
            prompt_file: Some(&prompt_path),
            env_vars: &[],
            background: false,
            restore_automatic_rename: false,
        });

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
    // build_layout_commands: --model applies to every claude pane
    // =========================================================================

    #[rstest]
    fn model_applied_to_all_claude_panes(two_claude_pane_layout: LayoutNode) {
        let layout = two_claude_pane_layout;

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: Some("opus"),
            prompt_file: None,
            env_vars: &[],
            background: false,
            restore_automatic_rename: false,
        });

        assert_eq!(
            commands,
            vec![
                cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"]),
                cmd(&["split-window", "-h", "-t", "1", "-c", "/tmp"]),
                cmd(&["select-pane", "-t", "1"]),
                cmd(&["send-keys", "-l", "--", "claude --model opus -p agent1"]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "2"]),
                cmd(&["send-keys", "-l", "--", "claude --model opus -p agent2"]),
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
        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: None,
            prompt_file: None,
            env_vars: &env_vars,
            background: false,
            restore_automatic_rename: false,
        });

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
    fn build_layout_commands_background_detaches_new_window_and_qualifies_pane_targets() {
        let layout = LayoutNode::Pane(PaneConfig {
            command: "claude".to_string(),
            focus: true,
        });

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: None,
            prompt_file: None,
            env_vars: &[],
            background: true,
            restore_automatic_rename: false,
        });

        // new-window must be detached so the attached client's view does not flip.
        assert_eq!(
            commands[0],
            cmd(&["new-window", "-d", "-t", "sess", "-c", "/tmp", "-n", "dev"])
        );

        // Every pane-targeting command must address the new window explicitly
        // via {session}:={window_name}.{pane}, never the bare pane index.
        let qualified = "sess:=dev.";
        for c in &commands[1..] {
            if matches!(
                c.args.first().map(String::as_str),
                Some("select-pane" | "send-keys" | "split-window")
            ) {
                let target_idx = c
                    .args
                    .iter()
                    .position(|a| a == "-t")
                    .expect("pane-addressing command without -t");
                let target = &c.args[target_idx + 1];
                assert!(
                    target.starts_with(qualified),
                    "expected target `{target}` to start with `{qualified}`",
                );
            }
        }
    }

    #[test]
    fn build_layout_commands_background_with_env_vars_places_new_window_at_expected_index() {
        // Documents that build_layout_commands emits exactly one set-environment
        // command per env_var before new-window, in the background + env_vars
        // combination that `a cc new --worktree --agent` exercises in production.
        let layout = LayoutNode::Pane(PaneConfig {
            command: "claude".to_string(),
            focus: true,
        });
        let env_vars = [("KEY1", "v1"), ("KEY2", "v2")];

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: None,
            prompt_file: None,
            env_vars: &env_vars,
            background: true,
            restore_automatic_rename: false,
        });

        assert_eq!(commands[env_vars.len()].args[0], "new-window");
    }

    #[test]
    fn build_layout_commands_foreground_keeps_unqualified_targets() {
        let layout = LayoutNode::Pane(PaneConfig {
            command: "claude".to_string(),
            focus: true,
        });

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: None,
            prompt_file: None,
            env_vars: &[],
            background: false,
            restore_automatic_rename: false,
        });

        // new-window is left attached (no `-d`).
        assert_eq!(
            commands[0],
            cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"])
        );

        // No previous-window restore command is appended.
        for c in &commands {
            assert_ne!(c.args.first().map(String::as_str), Some("select-window"));
        }
    }

    // =========================================================================
    // build_layout_commands: restore_automatic_rename
    // =========================================================================

    #[rstest]
    #[case::disabled(false, vec![
        cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"]),
        cmd(&["select-pane", "-t", "1"]),
        cmd(&["send-keys", "-l", "--", "claude"]),
        cmd(&["send-keys", "C-m"]),
        cmd(&["select-pane", "-t", "1"]),
    ])]
    #[case::enabled(true, vec![
        cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"]),
        cmd(&["set-option", "-w", "-t", "sess:=dev", "automatic-rename", "on"]),
        cmd(&["select-pane", "-t", "1"]),
        cmd(&["send-keys", "-l", "--", "claude"]),
        cmd(&["send-keys", "C-m"]),
        cmd(&["select-pane", "-t", "1"]),
    ])]
    fn restore_automatic_rename_inserts_set_option_right_after_new_window(
        #[case] restore_automatic_rename: bool,
        #[case] expected: Vec<TmuxCommand>,
    ) {
        let layout = LayoutNode::Pane(PaneConfig {
            command: "claude".to_string(),
            focus: true,
        });

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: None,
            prompt_file: None,
            env_vars: &[],
            background: false,
            restore_automatic_rename,
        });

        assert_eq!(commands, expected);
    }

    #[test]
    fn restore_automatic_rename_lands_outside_background_window_id_capture() {
        // execute_background_layout runs everything up to and including
        // new-window with `-P -F "#{window_id}"` appended to capture the real
        // window ID, so the restore command must come strictly after
        // new-window or it would be swept into that capture invocation
        // instead of running as its own command.
        let layout = LayoutNode::Pane(PaneConfig {
            command: "claude".to_string(),
            focus: true,
        });

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: None,
            prompt_file: None,
            env_vars: &[],
            background: true,
            restore_automatic_rename: true,
        });

        assert_eq!(
            find_new_window_index(&commands),
            Some(0),
            "new-window must remain the first command"
        );
        assert_eq!(
            commands[1],
            cmd(&[
                "set-option",
                "-w",
                "-t",
                "sess:=dev",
                "automatic-rename",
                "on"
            ])
        );
    }

    // =========================================================================
    // rewrite_pane_targets: swaps the session:=window_name. prefix for a
    // window-ID-based one, used once new-window's real ID is known
    // =========================================================================

    #[rstest]
    #[case::swaps_matching_prefix(
        vec![
            cmd(&["select-pane", "-t", "sess:=copier-update-v0.8.13.1"]),
            cmd(&[
                "send-keys",
                "-t",
                "sess:=copier-update-v0.8.13.1",
                "-l",
                "--",
                "claude",
            ]),
            cmd(&["send-keys", "-t", "sess:=copier-update-v0.8.13.1", "C-m"]),
            cmd(&["select-pane", "-t", "sess:=copier-update-v0.8.13.2"]),
        ],
        "sess:=copier-update-v0.8.13.",
        "@42.",
        vec![
            cmd(&["select-pane", "-t", "@42.1"]),
            cmd(&["send-keys", "-t", "@42.1", "-l", "--", "claude"]),
            cmd(&["send-keys", "-t", "@42.1", "C-m"]),
            cmd(&["select-pane", "-t", "@42.2"]),
        ]
    )]
    #[case::leaves_non_matching_args_untouched(
        vec![cmd(&["set-environment", "-u", "-t", "sess", "MY_VAR"])],
        "sess:=dev.",
        "@42.",
        vec![cmd(&["set-environment", "-u", "-t", "sess", "MY_VAR"])]
    )]
    #[case::leaves_window_level_restore_target_untouched(
        vec![cmd(&["set-option", "-w", "-t", "sess:=dev", "automatic-rename", "on"])],
        "sess:=dev.",
        "@42.",
        vec![cmd(&["set-option", "-w", "-t", "sess:=dev", "automatic-rename", "on"])]
    )]
    fn rewrite_pane_targets_cases(
        #[case] commands: Vec<TmuxCommand>,
        #[case] old_prefix: &str,
        #[case] new_prefix: &str,
        #[case] expected: Vec<TmuxCommand>,
    ) {
        assert_eq!(
            rewrite_pane_targets(&commands, old_prefix, new_prefix),
            expected
        );
    }

    // =========================================================================
    // find_new_window_index: locates new-window regardless of what precedes it
    // =========================================================================

    #[rstest]
    #[case::after_set_environment_commands(
        vec![
            cmd(&["set-environment", "-t", "sess", "KEY", "v"]),
            cmd(&["new-window", "-d", "-t", "sess", "-c", "/tmp", "-n", "dev"]),
            cmd(&["select-pane", "-t", "sess:=dev.1"]),
        ],
        Some(1)
    )]
    #[case::as_first_command(
        vec![cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"])],
        Some(0)
    )]
    #[case::absent(vec![cmd(&["select-pane", "-t", "1"])], None)]
    #[case::empty(vec![], None)]
    fn find_new_window_index_cases(
        #[case] commands: Vec<TmuxCommand>,
        #[case] expected: Option<usize>,
    ) {
        assert_eq!(find_new_window_index(&commands), expected);
    }

    #[test]
    fn execute_background_layout_errors_when_new_window_missing() {
        let commands = vec![cmd(&["select-pane", "-t", "1"])];

        let result = execute_background_layout(&commands, "sess", "win");

        assert_eq!(
            result.unwrap_err().to_string(),
            "new-window command not found in layout"
        );
    }

    #[test]
    fn empty_env_vars_no_set_environment() {
        let layout = LayoutNode::Pane(PaneConfig {
            command: "claude".to_string(),
            focus: true,
        });

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: None,
            prompt_file: None,
            env_vars: &[],
            background: false,
            restore_automatic_rename: false,
        });

        // First command should be new-window, not set-environment
        assert_eq!(
            commands[0],
            cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"])
        );
    }
}
